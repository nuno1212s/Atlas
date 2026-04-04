#![allow(dead_code)]
use atlas_core::execution::requests::{ReplyBatch, UpdateBatch};
use atlas_smr_application::app::{Application, Reply, Request};

#[inline(always)]
fn preemptive_execution<A, S>(
    application: &A,
    state: &mut S,
    batch: UpdateBatch<Request<A, S>>,
) -> ReplyBatch<Reply<A, S>>
where
    A: Application<S>,
{
    application.update_batch(state, batch)
}
