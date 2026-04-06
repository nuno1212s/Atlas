use atlas_common::channel::NoRetChannelErr;
use atlas_common::ordering::singular_tbo_queue::TSingleTboQueue;
use atlas_common::ordering::singular_tbo_queue::vec_single_tbo_queue::VSingleTBOQueue;
use atlas_common::ordering::{Orderable, SeqNo};
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
    /// Current sequence number of *confirmed* updates that have been committed.
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
        // The queue must start at initial_seq + 1: the first pending update to be
        // confirmed will be at that slot (position 0 relative to current_seq_no).
        let mut queue = VSingleTBOQueue::new();
        queue.reset_with_seq(initial_state.0.next());

        Self {
            current_state_seq_no: initial_state.0,
            current_confirmed_seq_no: initial_state.0,
            preemptive_state: initial_state.1,
            pending_permanent_update: queue,
        }
    }

    /// Install a new confirmed state received from the state transfer module
    pub fn install_confirmed_state(&mut self, confirmed_seq_no: SeqNo, confirmed_state: S) {
        self.preemptive_state = confirmed_state;
        self.current_confirmed_seq_no = confirmed_seq_no;
        self.current_state_seq_no = confirmed_seq_no;
        // Position the queue so that the next preemptive update (at confirmed_seq_no + 1)
        // lands at slot 0 and is immediately poppable.
        self.pending_permanent_update
            .reset_with_seq(confirmed_seq_no.next());
    }

    /// The rule which describes backtrack:
    /// We can backtrack to any seq such that: confirmed_seq < backtrack_seq < preemptive_seq
    /// Any backtrack_seq <= confirmed_seq is not an admissible backtrack as confirmed requests cannot be backtracked.
    ///
    /// When backtrack_seq > confirmed_seq, we must re-execute all preemptive requests confirmed_seq..backtrack_seq.
    pub fn backtrack(
        &mut self,
        application: &A,
        confirmed_seq_no: SeqNo,
        confirmed_state: S,
        backtracked_seq: SeqNo,
    ) -> Result<(), BacktrackError> {
        if backtracked_seq <= confirmed_seq_no {
            return Err(BacktrackError::BacktrackToConfirmedOrBelow {
                backtrack_seq: backtracked_seq,
                confirmed_seq: confirmed_seq_no,
            });
        }

        let batches = self.drain_pending_before(backtracked_seq);

        // Reset preemptive state to the confirmed baseline.
        self.preemptive_state = confirmed_state;
        self.current_confirmed_seq_no = confirmed_seq_no;
        self.current_state_seq_no = confirmed_seq_no;
        // Position queue at confirmed_seq_no + 1 so re-executed items land at slot 0.
        self.pending_permanent_update
            .reset_with_seq(confirmed_seq_no.next());

        // Re-execute kept batches via the normal preemptive update path, which handles
        // seq tracking and queue insertion consistently with handle_preemptive_update.
        for batch in batches {
            let seq = batch.seq_no();
            self.handle_preemptive_update(application, batch)
                .map_err(|_| BacktrackError::ReExecutionFailed { seq })?;
        }
        // After this, current_state_seq_no == backtracked_seq - 1 (or confirmed_seq_no if nothing
        // was re-executed), so the caller can immediately execute the update at backtracked_seq.
        Ok(())
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

        // Position queue at last_seq + 1 so the next preemptive update lands at slot 0.
        self.pending_permanent_update
            .reset_with_seq(last_seq.next());
    }

    /// Handle a directly-finalized update that was never preemptively executed.
    ///
    /// Executes the batch on the preemptive state to keep it in sync, advances both
    /// seq-no counters, and steps the pending-queue pointer past the slot that this
    /// batch occupies (since no entry was ever pushed for it).
    ///
    /// The returned [`ReplyBatch`] should be sent to clients immediately by the caller.
    pub fn handle_confirmed_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> Result<ReplyBatch<Reply<A, S>>, ConfirmedUpdateError> {
        let seq = update_batch.seq_no();

        if self.current_state_seq_no != self.current_confirmed_seq_no {
            return Err(ConfirmedUpdateError::PendingPreemptiveUpdates {
                confirmed_update_seq: seq,
                preemptive_seq: self.current_state_seq_no,
                confirmed_seq: self.current_confirmed_seq_no,
            });
        }

        let replies = application.update_batch(&mut self.preemptive_state, update_batch);

        self.current_state_seq_no = seq;
        self.current_confirmed_seq_no = seq;

        // No entry was pushed for this slot (no prior preemptive execution), so
        // just step the queue's seq pointer forward to account for the consumed slot.
        self.pending_permanent_update.advance_seq();

        Ok(replies)
    }

    pub fn handle_update_confirmed(
        &mut self,
        sequence_no: SeqNo,
    ) -> Result<PendingPermanentUpdate<A, S>, HandleUpdateConfirmedError> {
        // We can only confirm the next pending permanent update in order.
        if let Some(pending_permanent_update) = self.pending_permanent_update.peek() {
            if pending_permanent_update.0.seq_no() == sequence_no {
                let result = self.pending_permanent_update.pop().unwrap();

                self.pending_permanent_update.advance_seq();
                self.current_confirmed_seq_no = sequence_no;

                Ok(result)
            } else {
                Err(HandleUpdateConfirmedError::SeqMismatch {
                    expected: pending_permanent_update.0.seq_no(),
                    received: sequence_no,
                })
            }
        } else {
            Err(HandleUpdateConfirmedError::EmptyQueue {
                received: sequence_no,
            })
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

/// Returned by [`PreemptiveState::handle_confirmed_update`] when a directly-finalized
/// batch arrives while preemptive updates are still pending confirmation.
#[derive(Debug, Error)]
pub(super) enum ConfirmedUpdateError {
    #[error(
        "ConfirmedUpdate({confirmed_update_seq:?}) arrived while preemptive updates are still \
         pending (preemptive_seq={preemptive_seq:?}, confirmed_seq={confirmed_seq:?}). \
         All pending preemptive updates must be confirmed before a ConfirmedUpdate can be applied."
    )]
    PendingPreemptiveUpdates {
        confirmed_update_seq: SeqNo,
        preemptive_seq: SeqNo,
        confirmed_seq: SeqNo,
    },
}

/// Returned by [`PreemptiveState::handle_update_confirmed`] when the confirmation
/// for a preemptive update does not match what is at the head of the pending queue.
#[derive(Debug, Error)]
pub(super) enum HandleUpdateConfirmedError {
    #[error(
        "Preemptive confirmation for seq {received:?} does not match the front-of-queue \
         entry at seq {expected:?}"
    )]
    SeqMismatch { expected: SeqNo, received: SeqNo },
    #[error(
        "No pending preemptive update found for confirmation of seq {received:?}: queue is empty"
    )]
    EmptyQueue { received: SeqNo },
}

/// Returned by [`PreemptiveState::backtrack`] when the backtrack operation is invalid
/// or when re-execution of kept batches fails.
#[derive(Debug, Error)]
pub(super) enum BacktrackError {
    #[error(
        "Cannot backtrack to {backtrack_seq:?}: must be strictly above confirmed seq \
         {confirmed_seq:?}. Backtracking into already-confirmed history is not allowed."
    )]
    BacktrackToConfirmedOrBelow {
        backtrack_seq: SeqNo,
        confirmed_seq: SeqNo,
    },
    #[error(
        "Re-execution of batch {seq:?} failed during backtrack: \
         batches must arrive in sequential order"
    )]
    ReExecutionFailed { seq: SeqNo },
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

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_common::node_id::NodeId;
    use atlas_common::ordering::SeqNo;
    use atlas_core::execution::requests::{IncrementableUpdateBatch, UpdateBatch, UpdateInfo};
    use atlas_smr_application::app::Application;
    use atlas_smr_application::serialize::ApplicationData;

    // ---------------------------------------------------------------------------
    // Test fixtures
    // ---------------------------------------------------------------------------

    /// Minimal ApplicationData whose request/reply are both `u32`.
    struct TestData;

    impl ApplicationData for TestData {
        type Request = u32;
        type Reply = u32;

        fn serialize_request<W>(_w: W, _r: &u32) -> atlas_common::error::Result<()>
        where
            W: std::io::Write,
        {
            Ok(())
        }

        fn deserialize_request<R>(_r: R) -> atlas_common::error::Result<u32>
        where
            R: std::io::Read,
        {
            Ok(0)
        }

        fn serialize_reply<W>(_w: W, _r: &u32) -> atlas_common::error::Result<()>
        where
            W: std::io::Write,
        {
            Ok(())
        }

        fn deserialize_reply<R>(_r: R) -> atlas_common::error::Result<u32>
        where
            R: std::io::Read,
        {
            Ok(0)
        }
    }

    /// Simple counter application. State = running total (u32), request = value to add, reply = new total.
    struct TestApp;

    impl Application<u32> for TestApp {
        type AppData = TestData;

        fn initial_state() -> atlas_common::error::Result<u32> {
            Ok(0)
        }

        fn unordered_execution(&self, state: &u32, _req: u32) -> u32 {
            *state
        }

        fn update(&self, state: &mut u32, req: u32) -> u32 {
            *state += req;
            *state
        }
    }

    /// Build a single-request `UpdateBatch` at `seq` carrying `ops` as individual requests.
    fn make_batch(seq: u32, ops: &[u32]) -> UpdateBatch<u32> {
        let mut batch = UpdateBatch::new(SeqNo::from(seq));
        for &op in ops {
            let info =
                UpdateInfo::new_session_based(NodeId::from(0u32), SeqNo::ZERO, SeqNo::from(op));
            batch.add(info, op);
        }
        batch
    }

    fn new_state() -> PreemptiveState<u32, TestApp> {
        PreemptiveState::new((SeqNo::ZERO, 0u32))
    }

    // ---------------------------------------------------------------------------
    // Happy-path tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_initial_seq_is_zero() {
        let state = new_state();
        assert_eq!(state.sequence_number(), SeqNo::ZERO);
        assert_eq!(state.current_confirmed_seq_no, SeqNo::ZERO);
    }

    #[test]
    fn test_preemptive_update_advances_preemptive_seq() {
        let app = TestApp;
        let mut state = new_state();

        state
            .handle_preemptive_update(&app, make_batch(1, &[10]))
            .unwrap();
        assert_eq!(state.sequence_number(), SeqNo::from(1u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::ZERO);
    }

    #[test]
    fn test_preemptive_state_value_updates_correctly() {
        let app = TestApp;
        let mut state = new_state();

        state
            .handle_preemptive_update(&app, make_batch(1, &[10]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(2, &[20]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(3, &[5]))
            .unwrap();

        // State should be 10 + 20 + 5 = 35
        assert_eq!(state.preemptive_state, 35);
        assert_eq!(state.sequence_number(), SeqNo::from(3u32));
    }

    #[test]
    fn test_pure_preemptive_then_confirmed_in_order() {
        let app = TestApp;
        let mut state = new_state();

        // Three preemptive updates
        state
            .handle_preemptive_update(&app, make_batch(1, &[10]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(2, &[20]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(3, &[30]))
            .unwrap();

        // Confirm them in order; each should return the matching batch
        let p1 = state.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
        assert_eq!(p1.sequence_number(), SeqNo::from(1u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(1u32));

        let p2 = state.handle_update_confirmed(SeqNo::from(2u32)).unwrap();
        assert_eq!(p2.sequence_number(), SeqNo::from(2u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(2u32));

        let p3 = state.handle_update_confirmed(SeqNo::from(3u32)).unwrap();
        assert_eq!(p3.sequence_number(), SeqNo::from(3u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(3u32));
    }

    #[test]
    fn test_mix_preemptive_and_confirmed_interleaved() {
        let app = TestApp;
        let mut state = new_state();

        // Preemptive at 1, confirm, then preemptive at 2, confirm …
        state
            .handle_preemptive_update(&app, make_batch(1, &[1]))
            .unwrap();
        let _ = state.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(1u32));

        state
            .handle_preemptive_update(&app, make_batch(2, &[2]))
            .unwrap();
        let _ = state.handle_update_confirmed(SeqNo::from(2u32)).unwrap();
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(2u32));

        state
            .handle_preemptive_update(&app, make_batch(3, &[3]))
            .unwrap();
        let _ = state.handle_update_confirmed(SeqNo::from(3u32)).unwrap();
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(3u32));

        // Preemptive seq should also be 3, state = 1+2+3 = 6
        assert_eq!(state.sequence_number(), SeqNo::from(3u32));
        assert_eq!(state.preemptive_state, 6);
    }

    // ---------------------------------------------------------------------------
    // State transfer (install_confirmed_state)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_install_confirmed_state_resets_both_seq_nos() {
        let app = TestApp;
        let mut state = new_state();

        // Speculate ahead
        state
            .handle_preemptive_update(&app, make_batch(1, &[5]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(2, &[5]))
            .unwrap();

        // Receive a state snapshot at seq 10
        state.install_confirmed_state(SeqNo::from(10u32), 99u32);

        assert_eq!(state.sequence_number(), SeqNo::from(10u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(10u32));
        assert_eq!(state.preemptive_state, 99);
    }

    #[test]
    fn test_install_confirmed_state_then_resume_preemptive() {
        let app = TestApp;
        let mut state = new_state();

        state.install_confirmed_state(SeqNo::from(5u32), 50u32);

        // Can immediately push the next preemptive update
        state
            .handle_preemptive_update(&app, make_batch(6, &[10]))
            .unwrap();
        assert_eq!(state.sequence_number(), SeqNo::from(6u32));
        assert_eq!(state.preemptive_state, 60);
    }

    #[test]
    fn test_install_confirmed_state_then_confirm() {
        let app = TestApp;
        let mut state = new_state();

        state.install_confirmed_state(SeqNo::from(5u32), 50u32);
        state
            .handle_preemptive_update(&app, make_batch(6, &[10]))
            .unwrap();

        let confirmed = state.handle_update_confirmed(SeqNo::from(6u32)).unwrap();
        assert_eq!(confirmed.sequence_number(), SeqNo::from(6u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(6u32));
    }

    // ---------------------------------------------------------------------------
    // Catch-up
    // ---------------------------------------------------------------------------

    #[test]
    fn test_catch_up_advances_both_seq_nos() {
        let app = TestApp;
        let mut state = new_state();

        let batches = vec![
            make_batch(1, &[1]),
            make_batch(2, &[2]),
            make_batch(3, &[3]),
        ];
        state.handle_catch_up(&app, batches);

        assert_eq!(state.sequence_number(), SeqNo::from(3u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(3u32));
        // State value: 1 + 2 + 3 = 6
        assert_eq!(state.preemptive_state, 6);
    }

    #[test]
    fn test_catch_up_then_preemptive_continues() {
        let app = TestApp;
        let mut state = new_state();

        state.handle_catch_up(&app, vec![make_batch(1, &[10]), make_batch(2, &[10])]);

        // After catch-up to seq 2, new preemptive at seq 3 should succeed
        state
            .handle_preemptive_update(&app, make_batch(3, &[5]))
            .unwrap();
        assert_eq!(state.sequence_number(), SeqNo::from(3u32));
        assert_eq!(state.preemptive_state, 25); // 10 + 10 + 5
    }

    #[test]
    fn test_catch_up_then_confirm() {
        let app = TestApp;
        let mut state = new_state();

        state.handle_catch_up(&app, vec![make_batch(1, &[10])]);
        state
            .handle_preemptive_update(&app, make_batch(2, &[5]))
            .unwrap();

        let confirmed = state.handle_update_confirmed(SeqNo::from(2u32)).unwrap();
        assert_eq!(confirmed.sequence_number(), SeqNo::from(2u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(2u32));
    }

    // ---------------------------------------------------------------------------
    // Backtrack
    // ---------------------------------------------------------------------------

    #[test]
    fn test_backtrack_discards_all_pending_when_no_reexecution_needed() {
        let app = TestApp;
        let mut state = new_state();

        // Speculate: 1, 2, 3
        state
            .handle_preemptive_update(&app, make_batch(1, &[10]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(2, &[20]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(3, &[30]))
            .unwrap();

        // Backtrack to seq 1 (the update that caused conflict is at seq 1):
        // re-execute range is (confirmed=0, backtracked=1) exclusive = nothing.
        // All three pending items are discarded.
        state
            .backtrack(&app, SeqNo::ZERO, 0u32, SeqNo::from(1u32))
            .unwrap();

        assert_eq!(state.sequence_number(), SeqNo::ZERO);
        assert_eq!(state.current_confirmed_seq_no, SeqNo::ZERO);
        assert_eq!(state.preemptive_state, 0);

        // Can now re-execute from seq 1 forward
        state
            .handle_preemptive_update(&app, make_batch(1, &[7]))
            .unwrap();
        assert_eq!(state.preemptive_state, 7);
    }

    #[test]
    fn test_backtrack_reexecutes_items_before_backtrack_seq() {
        let app = TestApp;
        let mut state = new_state();

        // Preemptive updates at 1..=4
        for i in 1u32..=4 {
            state
                .handle_preemptive_update(&app, make_batch(i, &[i * 10]))
                .unwrap();
        }
        // State should be 10 + 20 + 30 + 40 = 100

        // Conflict detected at seq 3. Confirmed state is 0 (nothing confirmed yet).
        // Backtrack to seq 3:
        //   re-execute: seq 1 and seq 2 (seq < 3 AND > confirmed=0)
        //   discard:    seq 3 and seq 4
        state
            .backtrack(&app, SeqNo::ZERO, 0u32, SeqNo::from(3u32))
            .unwrap();

        // After backtrack, current_state_seq_no = 2 (last re-executed)
        assert_eq!(state.sequence_number(), SeqNo::from(2u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::ZERO);
        // State: re-executed 1*10 + 2*10 = 30 on fresh base 0
        assert_eq!(state.preemptive_state, 30);

        // Can now execute the conflicting update at seq 3
        state
            .handle_preemptive_update(&app, make_batch(3, &[99]))
            .unwrap();
        assert_eq!(state.preemptive_state, 30 + 99);
    }

    #[test]
    fn test_backtrack_with_confirmed_offset() {
        let app = TestApp;
        let mut state = new_state();

        // Preemptive at 1, confirm it
        state
            .handle_preemptive_update(&app, make_batch(1, &[5]))
            .unwrap();
        let _ = state.handle_update_confirmed(SeqNo::from(1u32)).unwrap();

        // Now speculate at 2, 3, 4
        state
            .handle_preemptive_update(&app, make_batch(2, &[5]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(3, &[5]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(4, &[5]))
            .unwrap();

        // Conflict at seq 3. Confirmed baseline is seq 1, state = 5.
        // Backtrack to seq 3:
        //   re-execute seq 2 (> confirmed=1 AND < backtracked=3)
        //   discard    seq 3 and 4
        state
            .backtrack(&app, SeqNo::from(1u32), 5u32, SeqNo::from(3u32))
            .unwrap();

        assert_eq!(state.sequence_number(), SeqNo::from(2u32));
        assert_eq!(state.preemptive_state, 5 + 5); // confirmed base 5 + re-exec seq2 (+5)

        // Re-execute the conflicting batch at seq 3
        state
            .handle_preemptive_update(&app, make_batch(3, &[100]))
            .unwrap();
        assert_eq!(state.preemptive_state, 5 + 5 + 100);
    }

    // ---------------------------------------------------------------------------
    // Error / invalid scenarios
    // ---------------------------------------------------------------------------

    #[test]
    fn test_future_request_returns_error() {
        let app = TestApp;
        let mut state = new_state();

        // Skip seq 1, go straight to seq 3
        let result = state.handle_preemptive_update(&app, make_batch(3, &[10]));
        assert!(
            matches!(result, Err(ExecuteUpdateError::FutureRequest(s, _)) if s == SeqNo::from(3u32)),
            "expected FutureRequest, got {:?}",
            result
        );
        // State must be unchanged
        assert_eq!(state.sequence_number(), SeqNo::ZERO);
    }

    #[test]
    fn test_backtracking_error_when_seq_behind_head() {
        let app = TestApp;
        let mut state = new_state();

        state
            .handle_preemptive_update(&app, make_batch(1, &[1]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(2, &[1]))
            .unwrap();

        // Seq 1 is behind the current head (seq 2) → Backtracking error
        let result = state.handle_preemptive_update(&app, make_batch(1, &[1]));
        assert!(
            matches!(result, Err(ExecuteUpdateError::Backtracking(s, _)) if s == SeqNo::from(1u32)),
            "expected Backtracking, got {:?}",
            result
        );
    }

    #[test]
    fn test_backtrack_to_confirmed_or_below_returns_error() {
        let app = TestApp;
        let mut state = new_state();

        state
            .handle_preemptive_update(&app, make_batch(1, &[1]))
            .unwrap();
        let _ = state.handle_update_confirmed(SeqNo::from(1u32)).unwrap();

        // Attempt to backtrack to the confirmed seq itself — not allowed.
        let err = state
            .backtrack(&app, SeqNo::from(1u32), 1u32, SeqNo::from(1u32))
            .unwrap_err();
        assert!(
            matches!(err, BacktrackError::BacktrackToConfirmedOrBelow {
                backtrack_seq,
                confirmed_seq,
            } if backtrack_seq == SeqNo::from(1u32) && confirmed_seq == SeqNo::from(1u32)),
            "expected BacktrackToConfirmedOrBelow, got {:?}",
            err
        );
    }

    // ---------------------------------------------------------------------------
    // Invariant: ConfirmedUpdate requires an empty preemptive queue
    //
    // A ConfirmedUpdate may only arrive when current_state_seq_no ==
    // current_confirmed_seq_no (no unconfirmed preemptive updates pending).
    // The tests below verify that handle_confirmed_update returns the correct
    // error variant for every illegal pattern.
    // ---------------------------------------------------------------------------

    /// Sending ConfirmedUpdate(2) while PreemptiveUpdate(1) is still unconfirmed
    /// is the simplest violation of the invariant.
    #[test]
    fn test_confirmed_update_errors_with_one_pending_preemptive() {
        let app = TestApp;
        let mut state = new_state();

        state
            .handle_preemptive_update(&app, make_batch(1, &[10]))
            .unwrap();

        // Illegal: seq 1 is still pending in the queue.
        let err = state
            .handle_confirmed_update(&app, make_batch(2, &[5]))
            .err()
            .unwrap();
        assert!(
            matches!(err, ConfirmedUpdateError::PendingPreemptiveUpdates {
                confirmed_update_seq,
                preemptive_seq,
                confirmed_seq,
            } if confirmed_update_seq == SeqNo::from(2u32)
              && preemptive_seq == SeqNo::from(1u32)
              && confirmed_seq == SeqNo::ZERO),
            "unexpected error: {:?}",
            err
        );
    }

    /// Multiple unconfirmed preemptive updates in the queue still return the
    /// correct error, regardless of how many are pending.
    #[test]
    fn test_confirmed_update_errors_with_multiple_pending_preemptive() {
        let app = TestApp;
        let mut state = new_state();

        state
            .handle_preemptive_update(&app, make_batch(1, &[1]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(2, &[2]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(3, &[3]))
            .unwrap();

        // Illegal: seq 1, 2, and 3 are all still unconfirmed.
        let err = state
            .handle_confirmed_update(&app, make_batch(4, &[4]))
            .err()
            .unwrap();
        assert!(
            matches!(err, ConfirmedUpdateError::PendingPreemptiveUpdates {
                confirmed_update_seq,
                preemptive_seq,
                confirmed_seq,
            } if confirmed_update_seq == SeqNo::from(4u32)
              && preemptive_seq == SeqNo::from(3u32)
              && confirmed_seq == SeqNo::ZERO),
            "unexpected error: {:?}",
            err
        );
    }

    /// Partially confirming preemptive updates is not enough — all must be
    /// confirmed before a ConfirmedUpdate is accepted.
    #[test]
    fn test_confirmed_update_errors_with_partially_confirmed_preemptive() {
        let app = TestApp;
        let mut state = new_state();

        state
            .handle_preemptive_update(&app, make_batch(1, &[1]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(2, &[2]))
            .unwrap();

        // Confirm only seq 1 — seq 2 is still pending.
        let _ = state.handle_update_confirmed(SeqNo::from(1u32)).unwrap();

        // Illegal: seq 2 is still unconfirmed.
        let err = state
            .handle_confirmed_update(&app, make_batch(3, &[3]))
            .err()
            .unwrap();
        assert!(
            matches!(err, ConfirmedUpdateError::PendingPreemptiveUpdates {
                confirmed_update_seq,
                preemptive_seq,
                confirmed_seq,
            } if confirmed_update_seq == SeqNo::from(3u32)
              && preemptive_seq == SeqNo::from(2u32)
              && confirmed_seq == SeqNo::from(1u32)),
            "unexpected error: {:?}",
            err
        );
    }

    // ---------------------------------------------------------------------------
    // Positive cases: ConfirmedUpdate IS legal when the queue is empty
    // ---------------------------------------------------------------------------

    /// ConfirmedUpdate is accepted when no preemptive updates are pending.
    #[test]
    fn test_confirmed_update_accepted_with_empty_queue() {
        let app = TestApp;
        let mut state = new_state();

        let replies = state
            .handle_confirmed_update(&app, make_batch(1, &[7]))
            .unwrap();
        assert_eq!(replies.len(), 1);
        assert_eq!(state.sequence_number(), SeqNo::from(1u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(1u32));
        assert_eq!(state.preemptive_state, 7);
    }

    /// ConfirmedUpdate is accepted after all pending preemptive updates have
    /// been confirmed (queue drained).
    #[test]
    fn test_confirmed_update_accepted_after_queue_fully_drained() {
        let app = TestApp;
        let mut state = new_state();

        state
            .handle_preemptive_update(&app, make_batch(1, &[10]))
            .unwrap();
        state
            .handle_preemptive_update(&app, make_batch(2, &[20]))
            .unwrap();

        // Confirm both — queue is now empty.
        let _ = state.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
        let _ = state.handle_update_confirmed(SeqNo::from(2u32)).unwrap();

        // Legal: no pending preemptive work.
        let replies = state
            .handle_confirmed_update(&app, make_batch(3, &[5]))
            .unwrap();
        assert_eq!(replies.len(), 1);
        assert_eq!(state.sequence_number(), SeqNo::from(3u32));
        assert_eq!(state.current_confirmed_seq_no, SeqNo::from(3u32));
        assert_eq!(state.preemptive_state, 35); // 10 + 20 + 5
    }

    /// Sequential ConfirmedUpdates are all accepted when no preemptive work exists.
    #[test]
    fn test_multiple_confirmed_updates_accepted_without_preemptive() {
        let app = TestApp;
        let mut state = new_state();

        let _ = state
            .handle_confirmed_update(&app, make_batch(1, &[3]))
            .unwrap();
        let _ = state
            .handle_confirmed_update(&app, make_batch(2, &[7]))
            .unwrap();
        let replies = state
            .handle_confirmed_update(&app, make_batch(3, &[10]))
            .unwrap();

        assert_eq!(replies.len(), 1);
        assert_eq!(state.sequence_number(), SeqNo::from(3u32));
        assert_eq!(state.preemptive_state, 20); // 3 + 7 + 10
    }
}
