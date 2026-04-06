use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::execution::requests::{ReplyBatch, UnorderedUpdateBatch, UpdateBatch, UpdateReply};
use atlas_smr_application::app::{Application, Reply, Request};
use rayon::ThreadPool;
use rayon::prelude::*;

pub struct ConfirmedRequestPipeline<S> {
    current_confirmed_seq_no: SeqNo,
    confirmed_state: S,
}

impl<S> Orderable for ConfirmedRequestPipeline<S> {
    fn sequence_number(&self) -> SeqNo {
        self.current_confirmed_seq_no
    }
}

impl<S> ConfirmedRequestPipeline<S> {
    pub fn new(seq: SeqNo, initial_state: S) -> Self {
        Self {
            current_confirmed_seq_no: seq,
            confirmed_state: initial_state,
        }
    }

    pub fn execute_update<A>(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> ReplyBatch<Reply<A, S>>
    where
        A: Application<S>,
    {
        let update_seq = update_batch.seq_no();

        let reply_batch = application.update_batch(&mut self.confirmed_state, update_batch);
        self.current_confirmed_seq_no = update_seq;

        reply_batch
    }

    pub fn execute_read<A>(
        &self,
        application: &A,
        update_batch: UnorderedUpdateBatch<Request<A, S>>,
        thread_pool: &ThreadPool,
    ) -> ReplyBatch<Reply<A, S>>
    where
        A: Application<S>,
        S: Send + Sync,
    {
        thread_pool.install(|| {
            update_batch
                .into_inner()
                .into_par_iter()
                .map(|request| {
                    let (info, req) = request.into_inner();

                    let result = application.unordered_execution(&self.confirmed_state, req);

                    UpdateReply::new(info, result)
                })
                .collect::<Vec<_>>()
                .into()
        })
    }

    pub fn take_state_snapshot(&self) -> (SeqNo, S)
    where
        S: Clone,
    {
        (self.sequence_number(), self.confirmed_state.clone())
    }

    pub fn install_state_message(&mut self, seq_no: SeqNo, state: S) {
        self.current_confirmed_seq_no = seq_no;
        self.confirmed_state = state;
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
    // Test fixtures (same minimal counter app as in preemptive_requests tests)
    // ---------------------------------------------------------------------------

    struct TestData;

    impl ApplicationData for TestData {
        type Request = u32;
        type Reply = u32;
        fn serialize_request<W>(_: W, _: &u32) -> atlas_common::error::Result<()>
        where
            W: std::io::Write,
        {
            Ok(())
        }
        fn deserialize_request<R>(_: R) -> atlas_common::error::Result<u32>
        where
            R: std::io::Read,
        {
            Ok(0)
        }
        fn serialize_reply<W>(_: W, _: &u32) -> atlas_common::error::Result<()>
        where
            W: std::io::Write,
        {
            Ok(())
        }
        fn deserialize_reply<R>(_: R) -> atlas_common::error::Result<u32>
        where
            R: std::io::Read,
        {
            Ok(0)
        }
    }

    struct TestApp;

    impl Application<u32> for TestApp {
        type AppData = TestData;
        fn initial_state() -> atlas_common::error::Result<u32> {
            Ok(0)
        }
        fn unordered_execution(&self, state: &u32, _: u32) -> u32 {
            *state
        }
        fn update(&self, state: &mut u32, req: u32) -> u32 {
            *state += req;
            *state
        }
    }

    fn make_batch(seq: u32, ops: &[u32]) -> UpdateBatch<u32> {
        let mut batch = UpdateBatch::new(SeqNo::from(seq));
        for &op in ops {
            batch.add(
                UpdateInfo::new_session_based(NodeId::from(0u32), SeqNo::ZERO, SeqNo::from(op)),
                op,
            );
        }
        batch
    }

    // ---------------------------------------------------------------------------
    // Tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_initial_seq_is_as_provided() {
        let pipeline = ConfirmedRequestPipeline::<u32>::new(SeqNo::ZERO, 0u32);
        assert_eq!(pipeline.sequence_number(), SeqNo::ZERO);
    }

    #[test]
    fn test_execute_update_advances_seq() {
        let app = TestApp;
        let mut pipeline = ConfirmedRequestPipeline::new(SeqNo::ZERO, 0u32);

        pipeline.execute_update(&app, make_batch(1, &[10]));
        assert_eq!(pipeline.sequence_number(), SeqNo::from(1u32));
    }

    #[test]
    fn test_execute_update_changes_state() {
        let app = TestApp;
        let mut pipeline = ConfirmedRequestPipeline::new(SeqNo::ZERO, 0u32);

        pipeline.execute_update(&app, make_batch(1, &[10]));
        pipeline.execute_update(&app, make_batch(2, &[20]));
        pipeline.execute_update(&app, make_batch(3, &[30]));

        let (seq, state) = pipeline.take_state_snapshot();
        assert_eq!(seq, SeqNo::from(3u32));
        assert_eq!(state, 60u32);
    }

    #[test]
    fn test_execute_update_returns_reply_batch() {
        let app = TestApp;
        let mut pipeline = ConfirmedRequestPipeline::new(SeqNo::ZERO, 0u32);

        let replies = pipeline.execute_update(&app, make_batch(1, &[5, 3]));
        // Two requests → two replies; values are 5 then 8 (running total)
        assert_eq!(replies.len(), 2);
        let inner = replies.into_inner();
        assert_eq!(*inner[0].reply(), 5u32);
        assert_eq!(*inner[1].reply(), 8u32);
    }

    #[test]
    fn test_take_state_snapshot_does_not_mutate() {
        let app = TestApp;
        let mut pipeline = ConfirmedRequestPipeline::new(SeqNo::ZERO, 0u32);
        pipeline.execute_update(&app, make_batch(1, &[42]));

        let (seq1, state1) = pipeline.take_state_snapshot();
        let (seq2, state2) = pipeline.take_state_snapshot();
        assert_eq!(seq1, seq2);
        assert_eq!(state1, state2);
        // Pipeline seq is unchanged
        assert_eq!(pipeline.sequence_number(), SeqNo::from(1u32));
    }

    #[test]
    fn test_install_state_message_replaces_state_and_seq() {
        let app = TestApp;
        let mut pipeline = ConfirmedRequestPipeline::new(SeqNo::ZERO, 0u32);
        pipeline.execute_update(&app, make_batch(1, &[10]));

        pipeline.install_state_message(SeqNo::from(50u32), 999u32);

        let (seq, state) = pipeline.take_state_snapshot();
        assert_eq!(seq, SeqNo::from(50u32));
        assert_eq!(state, 999u32);
    }

    #[test]
    fn test_execute_after_install_uses_new_state() {
        let app = TestApp;
        let mut pipeline = ConfirmedRequestPipeline::new(SeqNo::ZERO, 0u32);
        pipeline.install_state_message(SeqNo::from(10u32), 100u32);

        pipeline.execute_update(&app, make_batch(11, &[5]));
        let (seq, state) = pipeline.take_state_snapshot();
        assert_eq!(seq, SeqNo::from(11u32));
        assert_eq!(state, 105u32);
    }
}
