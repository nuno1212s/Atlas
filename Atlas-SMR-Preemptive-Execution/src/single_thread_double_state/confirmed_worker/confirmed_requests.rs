use crate::single_thread_double_state::confirmed_worker::comm_handles::ConfirmedChannels;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::execution::requests::{ReplyBatch, UpdateBatch};
use atlas_smr_application::app::{Application, Reply, Request};

pub(super) struct ConfirmedRequestPipeline<A, S>
where
    A: Application<S>,
{
    comm_handle: ConfirmedChannels<A, S>,

    current_confirmed_seq_no: SeqNo,
    confirmed_state: S,
}

impl<A, S> Orderable for ConfirmedRequestPipeline<A, S>
where
    A: Application<S>,
{
    fn sequence_number(&self) -> SeqNo {
        self.current_confirmed_seq_no
    }
}

impl<A, S> ConfirmedRequestPipeline<A, S>
where
    A: Application<S>,
{
    pub fn new(initial_state: S, handle: ConfirmedChannels<A, S>) -> Self {
        Self {
            comm_handle: handle,
            current_confirmed_seq_no: SeqNo::ZERO,
            confirmed_state: initial_state,
        }
    }

    pub fn execute_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> ReplyBatch<Reply<A, S>> {
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
