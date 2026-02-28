use crate::single_thread_double_state::preemptive_worker::comm_handles::PreemptiveChannels;
use atlas_common::ordering::singular_tbo_queue::TSingleTboQueue;
use atlas_common::ordering::singular_tbo_queue::vec_single_tbo_queue::VSingleTBOQueue;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::execution::requests::ReplyBatch;
use atlas_core::execution::requests::UpdateBatch;
use atlas_smr_application::app::{Application, Reply, Request};
use either::Either;

pub(super) struct PreemptiveRequestPipeline<S, A>
where
    A: Application<S>,
{
    current_state_seq_no: SeqNo,

    preemptive_state: S,

    pending_permanent_update: VSingleTBOQueue<PendingPermanentUpdate<A, S>>,

    channel_handles: PreemptiveChannels<A, S>,
}

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

impl<S, A> Orderable for PreemptiveRequestPipeline<S, A>
where
    A: Application<S>,
{
    fn sequence_number(&self) -> SeqNo {
        self.current_state_seq_no
    }
}

impl<S, A> PreemptiveRequestPipeline<S, A>
where
    A: Application<S>,
{
    pub fn new(handle: PreemptiveChannels<A, S>, initial_state: (SeqNo, S)) -> Self {
        Self {
            current_state_seq_no: initial_state.0,
            preemptive_state: initial_state.1,
            pending_permanent_update: VSingleTBOQueue::new(),
            channel_handles: handle,
        }
    }

    pub fn install_confirmed_state(&mut self, confirmed_state: S, confirmed_seq_no: SeqNo) {
        self.preemptive_state = confirmed_state;
        self.current_state_seq_no = confirmed_seq_no;

        self.pending_permanent_update = VSingleTBOQueue::new();
        self.pending_permanent_update
            .install_seq(confirmed_seq_no)
            .expect("Failed to install sequence number for pending permanent update queue");
    }

    pub fn handle_preemptive_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) where
        A: Application<S>,
    {
        match update_batch
            .sequence_number()
            .index(self.current_state_seq_no)
        {
            Either::Left(_) => {}
            Either::Right(1) => (),
            Either::Right(_) => panic!(
                "Preemptive update batch sequence number is too far ahead of current state sequence number"
            ),
        }

        let pending_copy = update_batch.clone();
        let replies = application.update_batch(&mut self.preemptive_state, update_batch);

        // Update the current state sequence number.
        self.current_state_seq_no = pending_copy.seq_no();

        self.pending_permanent_update
            .push(PendingPermanentUpdate(pending_copy, replies))
            .expect("Failed to push pending permanent update to the queue");
    }

    pub fn handle_update_confirmed(&mut self, sequence_no: SeqNo) -> PendingPermanentUpdate<A, S> {
        // We can only confirm the next pending permanent update in order.
        if let Some(pending_permanent_update) = self.pending_permanent_update.peek() {
            if pending_permanent_update.0.seq_no() == sequence_no {
                let result = self.pending_permanent_update.pop().unwrap();

                self.pending_permanent_update.advance_seq();

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
