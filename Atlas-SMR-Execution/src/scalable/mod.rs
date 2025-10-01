#![allow(dead_code)]

use std::collections::BTreeMap;
use std::time::Instant;

use rayon::prelude::*;
use rayon::ThreadPool;

use crate::metric::{
    EXECUTION_TIME_TAKEN_ID, OPERATIONS_EXECUTED_PER_SECOND_ID, UNORDERED_EXECUTION_TIME_TAKEN_ID,
    UNORDERED_OPS_PER_SECOND_ID,
};
use crate::scalable::execution_unit::{
    progress_collision_state, CollisionState, ExecutionResult, ExecutionUnit,
};
use atlas_common::channel;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_metrics::metrics::{metric_duration, metric_increment};
use atlas_smr_application::app::{
    Application, BatchReplies, Reply, Request, UnorderedBatch, UpdateBatch, UpdateReply,
};

pub mod divisible_state_exec;
mod execution_unit;
pub mod monolithic_exec;

/// How many threads should we use in the execution threadpool
const THREAD_POOL_THREADS: u32 = 4;

struct Access {
    column: String,
    key: Vec<u8>,
    access_type: AccessType,
}

/// Types of accesses to data stored in the state
#[derive(Copy, Clone, Debug, PartialOrd, PartialEq, Eq, Ord)]
enum AccessType {
    Read,
    Write,
    Delete,
}

/// A trait defining the CRUD operations required to be implemented for a given state in order
/// for it to be utilized as a scalable state.
pub trait CRUDState: Send {
    /// Read an entry from the state
    fn read(&self, column: &str, key: &[u8]) -> Option<Vec<u8>>;

    /// Create a new entry in the state
    fn create(&mut self, column: &str, key: &[u8], value: &[u8]) -> bool;

    /// Update an entry in the state
    /// Returns the previous value that was stored in the state
    fn update(&mut self, column: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>>;

    /// Delete an entry in the state
    fn delete(&mut self, column: &str, key: &[u8]) -> Option<Vec<u8>>;
}

/// A trait defining the methods required for an application to be scalable
pub trait ScalableApp<S>: Application<S> + Sync
where
    S: CRUDState,
{
    /// This execution method takes a dynamic reference to the state. This is because
    /// We will pass it an ExecutionUnit which is not S, so it must handle any state that
    /// implements CRUDState.
    /// This operation must yield the same result as the ordered execution of the same request
    /// for this to correctly function.
    fn speculatively_execute(
        &self,
        state: &mut impl CRUDState,
        request: Request<Self, S>,
    ) -> Reply<Self, S>;
}

/// Execute the given batch in a scalable manner, utilizing a thread pool, performing collision analysis
fn scalable_execution<'a, A, S>(
    thread_pool: &mut ThreadPool,
    application: &A,
    state: &mut S,
    batch: UpdateBatch<Request<A, S>>,
) -> BatchReplies<Reply<A, S>>
where
    A: ScalableApp<S>,
    S: CRUDState + Sync + Send + 'a,
{
    let seq_no = batch.sequence_number();

    let mut execution_results = BTreeMap::new();

    let mut replies = BatchReplies::with_capacity(batch.len());

    let (tx, rx) = channel::sync::new_bounded_sync(batch.len(), None::<String>);

    let updates = batch
        .into_inner()
        .into_iter()
        .enumerate()
        .collect::<Vec<_>>();

    let chunk_size = updates.len() / (thread_pool.current_num_threads());

    let mut collision_state = CollisionState::default();

    thread_pool.scope(|scope| {
        // Start the workers that will process all the updates.
        // These updates are kept mostly in order
        for chunk in 0..updates.chunks(chunk_size).count() {
            let state = &*state;
            let tx = tx.clone();
            let updates = &updates;

            scope.spawn(move |_scope| {
                updates
                    .chunks(chunk_size)
                    .nth(chunk)
                    .unwrap()
                    .iter()
                    .for_each(|(pos, request)| {
                        let request = request.clone();

                        let mut exec_unit = ExecutionUnit {
                            seq_no,
                            position_in_batch: *pos,
                            accesses: Default::default(),
                            cache: Default::default(),
                            state_reference: state,
                        };

                        let reply = application
                            .speculatively_execute(&mut exec_unit, request.operation().clone());

                        tx.send((
                            *pos,
                            exec_unit.complete(),
                            UpdateReply::init(
                                request.from(),
                                request.session_id(),
                                request.operation_id(),
                                reply,
                            ),
                        ))
                        .unwrap();
                    });
            });
        }

        while let Ok((pos, exec_unit, reply)) = rx.recv() {
            progress_collision_state(&mut collision_state, &exec_unit);

            execution_results.insert(pos, (exec_unit, reply));
        }
    });

    for collision in collision_state.collisions {
        // Discard of the results since we have they have collided
        execution_results.remove(&collision);

        let (_, update) = &updates[collision];

        // Re execute them in the correct ordering.
        // This is guaranteed because the collisions is an ordered set
        let app_reply = application.update(state, update.operation().clone());

        replies.add(
            update.from(),
            update.session_id(),
            update.operation_id(),
            app_reply,
        );
    }

    let mut to_apply = Vec::with_capacity(execution_results.len());

    for (_pos, (exec_unit, reply)) in execution_results {
        to_apply.push(exec_unit);

        replies.push(reply);
    }

    apply_results_to_state(state, to_apply);

    replies
}

/// Unordered execution scales better than ordered execution and does not require collision verification
pub(super) fn scalable_unordered_execution<A, S>(
    thread_pool: &mut ThreadPool,
    application: &A,
    state: &S,
    batch: UnorderedBatch<Request<A, S>>,
) -> BatchReplies<Reply<A, S>>
where
    A: Application<S>,
    S: Send + Sync,
{
    thread_pool.install(move || {
        let replies = batch
            .into_inner()
            .into_par_iter()
            .map(|request| {
                let (from, session, op_id, op) = request.into_inner();

                let reply = speculatively_execute_unordered(application, state, op);

                UpdateReply::init(from, session, op_id, reply)
            })
            .collect::<Vec<_>>();

        replies.into()
    })
}

/// Apply the given results of execution to the state
fn apply_results_to_state<S>(state: &mut S, execution_units: Vec<ExecutionResult>)
where
    S: CRUDState,
{
    for unit in execution_units {
        unit.alterations().iter().for_each(|alteration| {
            match alteration.access_type {
                AccessType::Read => {
                    // Ignore read accesses as they do not affect the state
                }
                AccessType::Write => {
                    //TODO: If this repeats various write accesses to the same key,
                    // Reduce them all into a single access
                    if let Some(value) = unit.cache().get(&alteration.column) {
                        if let Some(value) = value.get(&alteration.key) {
                            state.update(&alteration.column, &alteration.key, value);
                        }
                    }
                }
                AccessType::Delete => {
                    state.delete(&alteration.column, &alteration.key);
                }
            }
        });
    }
}

#[inline(always)]
/// Speculatively execute an unordered operation
fn speculatively_execute_unordered<A, S>(
    application: &A,
    state: &S,
    request: Request<A, S>,
) -> Reply<A, S>
where
    A: Application<S>,
{
    application.unordered_execution(state, request)
}

#[inline(always)]
pub(crate) fn sc_execute_unordered_op_batch<A, S>(
    thread_pool: &mut ThreadPool,
    application: &A,
    state: &S,
    batch: UnorderedBatch<Request<A, S>>,
) -> BatchReplies<Reply<A, S>>
where
    A: Application<S>,
    S: Send + Sync,
{
    let op_count = batch.len() as u64;
    let exec_start_time = Instant::now();

    let reply = scalable_unordered_execution(thread_pool, application, state, batch);

    metric_increment(UNORDERED_OPS_PER_SECOND_ID, Some(op_count));
    metric_duration(UNORDERED_EXECUTION_TIME_TAKEN_ID, exec_start_time.elapsed());

    reply
}

fn sc_execute_op_batch<A, S>(
    thread_pool: &mut ThreadPool,
    application: &A,
    state: &mut S,
    batch: UpdateBatch<Request<A, S>>,
) -> (SeqNo, BatchReplies<Reply<A, S>>)
where
    A: ScalableApp<S>,
    S: CRUDState + Send + Sync,
{
    let seq_no = batch.sequence_number();
    let operations = batch.len() as u64;

    let start = Instant::now();

    let reply_batch = scalable_execution(thread_pool, application, state, batch);

    metric_duration(EXECUTION_TIME_TAKEN_ID, start.elapsed());
    metric_increment(OPERATIONS_EXECUTED_PER_SECOND_ID, Some(operations));

    (seq_no, reply_batch)
}

impl Access {
    fn init(column: &str, key: Vec<u8>, access_type: AccessType) -> Self {
        Self {
            column: column.to_string(),
            key,
            access_type,
        }
    }
}

impl AccessType {
    fn is_collision(&self, access_type: &AccessType) -> bool {
        match (self, access_type) {
            // Write - Anything is a collision
            (AccessType::Write, _) | (_, AccessType::Write) => true,
            // Delete - Anything is also a collision
            (AccessType::Delete, _) | (_, AccessType::Delete) => true,
            (_, _) => false,
        }
    }
}
