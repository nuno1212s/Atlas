use atlas_common::channel::NoRetChannelErr;
use atlas_common::ordering::singular_tbo_queue::TSingleTboQueue;
use atlas_common::ordering::singular_tbo_queue::vec_single_tbo_queue::VSingleTBOQueue;
use atlas_common::ordering::{InvalidSeqNo, Orderable, SeqNo};
use atlas_core::execution::requests::ReplyBatch;
use atlas_core::execution::requests::UpdateBatch;
use atlas_smr_application::app::{Application, Reply, Request};
use either::Either;
use std::fmt::{Debug, Formatter};
use thiserror::Error;

pub(super) struct PendingPermanentUpdate<A, S>(UpdateBatch<Request<A, S>>, ReplyBatch<Reply<A, S>>)
where
    A: Application<S>;

impl<A, S> Orderable for PendingPermanentUpdate<A, S>
where
    A: Application<S>,
{
    fn sequence_number(&self) -> SeqNo {
        self.0.sequence_number()
    }
}

impl<A, S> PendingPermanentUpdate<A, S>
where
    A: Application<S>,
{
    #[allow(dead_code)]
    pub fn new(
        update_batch: UpdateBatch<Request<A, S>>,
        reply_batch: ReplyBatch<Reply<A, S>>,
    ) -> Self {
        Self(update_batch, reply_batch)
    }

    #[allow(clippy::type_complexity)]
    pub fn into_inner(self) -> (UpdateBatch<Request<A, S>>, ReplyBatch<Reply<A, S>>) {
        (self.0, self.1)
    }
}

pub(super) struct PreemptiveState<S, A>
where
    A: Application<S>,
{
    /// Current sequence number of *preemptive* updates that have been executed.
    current_state_seq_no: SeqNo,
    ///
    current_confirmed_seq_no: SeqNo,
    /// Current state, at [current_state_seq_no] sequence number, which has been executed with preemptive updates.
    preemptive_state: S,

    pending_permanent_update: VSingleTBOQueue<PendingPermanentUpdate<A, S>>,
}

impl<S, A> Orderable for PreemptiveState<S, A>
where
    A: Application<S>,
{
    fn sequence_number(&self) -> SeqNo {
        self.current_state_seq_no
    }
}

impl<S, A> PreemptiveState<S, A>
where
    A: Application<S>,
{
    pub fn new(initial_state: (SeqNo, S)) -> Self {
        Self {
            current_state_seq_no: initial_state.0,
            current_confirmed_seq_no: initial_state.0,
            preemptive_state: initial_state.1,
            pending_permanent_update: VSingleTBOQueue::new(),
        }
    }

    /// Install a new confirmed state received from the state transfer module
    pub fn install_confirmed_state(&mut self, confirmed_seq_no: SeqNo, confirmed_state: S) {
        self.preemptive_state = confirmed_state;
        self.current_confirmed_seq_no = confirmed_seq_no;
        self.current_state_seq_no = confirmed_seq_no;
        self.pending_permanent_update
            .reset_with_seq(confirmed_seq_no);
    }


    /// The rule which describes backtrack:
    /// We can backtrack to any seq such that: confirmed_seq <= backtrack_seq < preemptive_seq
    /// Any backtrack_seq < confirmed_seq is not an admissible backtrack as confirmed requests cannot be backtracked.
    ///
    /// When backtrack_seq > confirmed_seq, we must re-execute all preemptive requests confirmed_seq..backtrack_seq.
    pub fn backtrack(&mut self, application: &A, confirmed_seq_no: SeqNo, confirmed_state: S, backtracked_seq: SeqNo) {
        let batches = self.drain_pending_before(backtracked_seq);

        // Reset preemptive state to the confirmed baseline.
        self.preemptive_state = confirmed_state;
        self.current_confirmed_seq_no = confirmed_seq_no;
        self.current_state_seq_no = confirmed_seq_no;
        self.pending_permanent_update.reset_with_seq(confirmed_seq_no);

        // Re-execute kept batches via the normal preemptive update path, which handles
        // seq tracking and queue insertion consistently with handle_preemptive_update.
        for batch in batches {
            self.handle_preemptive_update(application, batch)
                .expect("Backtrack re-execution failed: batches must arrive in sequential order");
        }
        // After this, current_state_seq_no == backtracked_seq - 1 (or confirmed_seq_no if nothing
        // was re-executed), so the caller can immediately execute the update at backtracked_seq.
    }

    /// Drains the pending queue and returns the [`UpdateBatch`]es with seq < `before_seq`.
    /// Batches at seq >= `before_seq` are discarded as they were built on the wrong speculative path.
    fn drain_pending_before(&mut self, before_seq: SeqNo) -> Vec<UpdateBatch<Request<A, S>>> {
        let limit = self.current_state_seq_no;
        let mut batches = Vec::new();

        loop {
            if self.pending_permanent_update.sequence_number() > limit {
                break;
            }

            if let Some(item) = self.pending_permanent_update.pop() {
                let (batch, _replies) = item.into_inner();

                if batch.sequence_number() < before_seq {
                    batches.push(batch);
                }
                // Batches at seq >= before_seq are dropped
            }

            self.pending_permanent_update.advance_seq();
        }

        batches
    }

    pub fn handle_preemptive_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> Result<(), ExecuteUpdateError<Request<A, S>>>
    where
        A: Application<S>,
    {
        match update_batch
            .sequence_number()
            .index(self.current_state_seq_no)
        {
            Either::Left(_) => {
                return Err(ExecuteUpdateError::Backtracking(
                    update_batch.sequence_number(),
                    update_batch,
                ));
            }
            Either::Right(1) => (),
            Either::Right(_) => {
                return Err(ExecuteUpdateError::FutureRequest(
                    update_batch.sequence_number(),
                    self.current_state_seq_no,
                ));
            }
        }

        let pending_copy = update_batch.clone();
        let replies = application.update_batch(&mut self.preemptive_state, update_batch);

        // Update the current state sequence number.
        self.current_state_seq_no = pending_copy.seq_no();

        self.pending_permanent_update
            .push(PendingPermanentUpdate(pending_copy, replies))
            .expect("Failed to push pending permanent update to the queue");

        Ok(())
    }

    pub fn handle_catch_up(
        &mut self,
        application: &A,
        batches: impl IntoIterator<Item = UpdateBatch<Request<A, S>>>,
    ) {
        let mut last_seq = self.current_confirmed_seq_no;

        for batch in batches {
            let seq = batch.seq_no();
            // Execute directly and discard replies — no client notifications needed for catch-up
            let _ = application.update_batch(&mut self.preemptive_state, batch);

            last_seq = seq;
        }

        self.current_state_seq_no = last_seq;
        self.current_confirmed_seq_no = last_seq;

        self.pending_permanent_update
            .reset_with_seq(self.current_state_seq_no);
    }

    pub fn handle_update_confirmed(&mut self, sequence_no: SeqNo) -> PendingPermanentUpdate<A, S> {
        // We can only confirm the next pending permanent update in order.
        if let Some(pending_permanent_update) = self.pending_permanent_update.peek() {
            if pending_permanent_update.0.seq_no() == sequence_no {
                let result = self.pending_permanent_update.pop().unwrap();

                self.pending_permanent_update.advance_seq();
                self.current_confirmed_seq_no = sequence_no;

                result
            } else {
                panic!(
                    "Confirmed update batch sequence number does not match the next pending permanent update"
                );
            }
        } else {
            panic!("Preemptive update batch sequence number is not found");
        }
    }
}

#[derive(Error)]
pub(super) enum ExecuteUpdateError<R> {
    #[error("Backtracked execution. Need new state {0:?}")]
    Backtracking(SeqNo, UpdateBatch<R>),
    #[error("Received a request which is ahead of our current execution {0:?} (current {1:?}")]
    FutureRequest(SeqNo, SeqNo),
    #[error("Channel error {0:?}")]
    ChannelErr(#[from] NoRetChannelErr),
}

impl<R> Debug for ExecuteUpdateError<R> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteUpdateError::Backtracking(seq_no, update_batch) => f
                .debug_tuple("Backtracking")
                .field(seq_no)
                .field(&format!("Update batch with {} updates", update_batch.len()))
                .finish(),
            ExecuteUpdateError::FutureRequest(seq, current_seq) => f
                .debug_tuple("FutureRequest")
                .field(seq)
                .field(current_seq)
                .finish(),
            ExecuteUpdateError::ChannelErr(err) => {
                f.debug_tuple("ChannelError").field(err).finish()
            }
        }
    }
}
