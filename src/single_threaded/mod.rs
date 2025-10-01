use crate::metric::{
    EXECUTION_TIME_TAKEN_ID, OPERATIONS_EXECUTED_PER_SECOND_ID, UNORDERED_EXECUTION_TIME_TAKEN_ID,
    UNORDERED_OPS_PER_SECOND_ID,
};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_metrics::metrics::{metric_duration, metric_increment};
use atlas_smr_application::app::{
    Application, BatchReplies, Reply, Request, UnorderedBatch, UpdateBatch,
};
use std::time::Instant;
use tracing::instrument;

pub mod divisible_state_exec;
pub mod monolithic_executor;

trait UnorderedExecutor<A, S> {
    fn execute_unordered(
        &mut self,
        batch: UnorderedBatch<Request<A, S>>,
    ) -> BatchReplies<Reply<A, S>>
    where
        A: Application<S>;
}

#[instrument(skip_all, fields(request_count = batch.len()))]
fn st_execute_op_batch<A, S>(
    application: &A,
    state: &mut S,
    batch: UpdateBatch<Request<A, S>>,
) -> (SeqNo, BatchReplies<Reply<A, S>>)
where
    A: Application<S>,
{
    let seq_no = batch.sequence_number();
    let operations = batch.len() as u64;

    let start = Instant::now();

    let reply_batch = application.update_batch(state, batch);

    metric_duration(EXECUTION_TIME_TAKEN_ID, start.elapsed());
    metric_increment(OPERATIONS_EXECUTED_PER_SECOND_ID, Some(operations));

    (seq_no, reply_batch)
}

#[instrument(skip_all, fields(request_count = batch.len()))]
fn st_execute_unordered_op_batch<A, S>(
    application: &A,
    state: &S,
    batch: UnorderedBatch<Request<A, S>>,
) -> BatchReplies<Reply<A, S>>
where
    A: Application<S>,
{
    let exec_start_time = Instant::now();
    let operations = batch.len() as u64;

    let reply_batch = application.unordered_batched_execution(state, batch);

    metric_increment(UNORDERED_OPS_PER_SECOND_ID, Some(operations));
    metric_duration(UNORDERED_EXECUTION_TIME_TAKEN_ID, exec_start_time.elapsed());

    reply_batch
}
