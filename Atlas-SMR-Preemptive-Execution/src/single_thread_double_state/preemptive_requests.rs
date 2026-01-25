use std::collections::VecDeque;

use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::execution::requests::UpdateBatch;
use atlas_smr_application::{
    app::{Application, Reply, Request}
};
use atlas_core::execution::requests::ReplyBatch;
use either::Either;

pub(super) struct PreemptiveRequestPipeline<S, A>
where
    A: Application<S>,
{
    current_state_seq_no: SeqNo,

    preemptive_state: S,

    pending_permanent_update: VecDeque<(UpdateBatch<Request<A, S>>, ReplyBatch<Reply<A, S>>)>,
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
    pub fn new(initial_state: S) -> Self {
        Self {
            current_state_seq_no: SeqNo::ZERO,
            preemptive_state: initial_state,
            pending_permanent_update: VecDeque::new(),
        }
    }

    pub fn handle_preemptive_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) where
        A: Application<S>,
    {
        match update_batch.sequence_number().index(self.current_state_seq_no) {
            Either::Left(_) => {
                
            },
            Either::Right(1) => (),
            Either::Right(_) => panic!("Preemptive update batch sequence number is too far ahead of current state sequence number"),
        }

        let pending_copy = update_batch.clone();
        let replies = application.update_batch(&mut self.preemptive_state, update_batch);

        // Update the current state sequence number.
        self.current_state_seq_no = pending_copy.seq_no();
        self.pending_permanent_update.push_back((pending_copy, replies));
    }

    pub fn handle_update_confirmed(&mut self, sequence_no: SeqNo) -> Option<(UpdateBatch<Request<A, S>>, ReplyBatch<Reply<A, S>>)> {


        if let Some((pending_update, _)) = self.pending_permanent_update.front() {
            if pending_update.seq_no() == sequence_no {
                return self.pending_permanent_update.pop_front();
            }
        }

        None
    }
}
