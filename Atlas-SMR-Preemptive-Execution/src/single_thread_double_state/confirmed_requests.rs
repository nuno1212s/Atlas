use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::execution::requests::{ReplyBatch, UpdateBatch};
use atlas_smr_application::app::{Application, Reply, Request};

pub(super) struct ConfirmedRequestPipeline<S> {
    current_confirmed_seq_no: SeqNo,
    confirmed_state: S,
}

impl<S> Orderable for ConfirmedRequestPipeline<S> {
    fn sequence_number(&self) -> SeqNo {
        self.current_confirmed_seq_no
    }
}

impl<S> ConfirmedRequestPipeline<S> {
    pub fn new(initial_state: S) -> Self {
        Self {
            current_confirmed_seq_no: SeqNo::ZERO,
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

    pub fn take_state_snapshot(&self) -> (SeqNo, S)
    where
        S: Clone,
    {
        (self.sequence_number(), self.confirmed_state.clone())
    }
}
