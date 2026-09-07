use crate::metric::{
    OPERATIONS_EXECUTED_PER_SECOND_ID, REORDER_BUFFER_SIZE_ID, REORDER_STAGED_COUNT_ID,
    SCALABLE_COLLISION_COUNT_ID, SCALABLE_COLLISION_RATE_ID, SCALABLE_OPS_PER_BATCH_ID,
    SCALABLE_PREEMPTIVE_EXECUTION_TIME_ID, SPECULATION_FALLBACK_COUNT_ID,
};
use crate::scalable_crud::execution_unit::{
    CollisionState, ParallelExecutionUnit, progress_collision_state,
};
use crate::single_threaded_crud::caching_state::{
    AccumulatedCache, CachingState, apply_delta_to_state, merge_delta_into,
    rebuild_accumulated_cache,
};
use crate::single_threaded_crud::pending_state::{MAX_STAGED_BATCHES, PreemptiveOutcome};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::execution::requests::{
    IncrementableUpdateBatch, ReplyBatch, UpdateBatch, UpdateInfo,
};
use atlas_metrics::metrics::{metric_duration, metric_increment, metric_store_count};
use atlas_smr_application::app::{Application, Reply, Request};
use atlas_smr_execution::crud_states::{Access, CRUDApplication, CRUDState};
use rayon::ThreadPool;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::fmt::{Debug, Formatter};
use std::time::Instant;
use thiserror::Error;

/// One entry in the parallel speculative execution result:
/// `(batch_position, accesses, local_cache_delta, update_info, op_clone, reply)`.
type ParallelResultEntry<A, S> = (
    usize,
    Vec<Access>,
    AccumulatedCache,
    UpdateInfo,
    Request<A, S>,
    Reply<A, S>,
);

// ---------------------------------------------------------------------------
// PendingCachedUpdate (same structure as single_threaded_crud version)
// ---------------------------------------------------------------------------

pub(super) struct PendingCachedUpdate<A, S>
where
    A: Application<S>,
{
    pub(super) batch: UpdateBatch<Request<A, S>>,
    pub(super) replies: ReplyBatch<Reply<A, S>>,
    pub(super) delta: AccumulatedCache,
    pub(super) speculated_at: Instant,
}

impl<A, S> Orderable for PendingCachedUpdate<A, S>
where
    A: Application<S>,
{
    fn sequence_number(&self) -> SeqNo {
        self.batch.sequence_number()
    }
}

// ---------------------------------------------------------------------------
// ScalableCachingPreemptiveState
// ---------------------------------------------------------------------------

/// State machine for the scalable CRUD preemptive executor.
///
/// The preemptive update path runs requests in parallel (via a rayon thread pool) through
/// per-request `ParallelExecutionUnit` proxies, then detects collisions and re-executes
/// conflicting requests sequentially. All other paths (confirmation, backtrack, catch-up,
/// state transfer) are identical to `CachingPreemptiveState`.
pub(super) struct ScalableCachingPreemptiveState<S, A>
where
    A: CRUDApplication<S>,
    S: CRUDState + Sync,
{
    confirmed_state: S,
    /// Sequence number the next confirmation is expected to carry.
    next_confirmed: SeqNo,
    /// Sequence number the next preemptive update is expected to carry.
    next_preemptive: SeqNo,
    accumulated_cache: AccumulatedCache,
    pending: VecDeque<PendingCachedUpdate<A, S>>,
    /// Batches that arrived ahead of `next_preemptive`, held until the gap closes.
    staged: BTreeMap<SeqNo, UpdateBatch<Request<A, S>>>,
    thread_pool: ThreadPool,
}

impl<S, A> ScalableCachingPreemptiveState<S, A>
where
    A: CRUDApplication<S> + Sync,
    S: CRUDState + Sync,
    Request<A, S>: Clone,
{
    pub(super) fn new(initial_state: (SeqNo, S), thread_pool: ThreadPool) -> Self {
        Self {
            confirmed_state: initial_state.1,
            next_confirmed: initial_state.0,
            next_preemptive: initial_state.0,
            accumulated_cache: AccumulatedCache::default(),
            pending: VecDeque::new(),
            staged: BTreeMap::new(),
            thread_pool,
        }
    }

    pub(super) fn next_confirmed_seq(&self) -> SeqNo {
        self.next_confirmed
    }

    pub(super) fn next_preemptive_seq(&self) -> SeqNo {
        self.next_preemptive
    }

    pub(super) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub(super) fn staged_count(&self) -> usize {
        self.staged.len()
    }

    pub(super) fn pending_front_seq(&self) -> Option<SeqNo> {
        self.pending.front().map(|p| p.sequence_number())
    }

    pub(super) fn confirmed_state(&self) -> &S {
        &self.confirmed_state
    }

    // -----------------------------------------------------------------------
    // Preemptive update — parallel execution with collision detection
    // -----------------------------------------------------------------------

    /// Speculatively execute a batch using the two-layer parallel approach:
    ///
    /// 1. Each request runs in parallel through a `ParallelExecutionUnit`.
    /// 2. Collision detection identifies within-batch conflicts.
    /// 3. Non-colliding results are merged into a `batch_delta` in order.
    /// 4. Colliding requests are re-executed sequentially against (accumulated + batch_delta).
    /// 5. The final `batch_delta` goes into the pending queue.
    pub(super) fn handle_preemptive_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> Result<PreemptiveOutcome, PreemptiveError<A, S>> {
        let seq = update_batch.sequence_number();

        // `SeqNo` compares by raw value under `Ord`; its `PartialOrd` goes through the
        // wrap-aware `index`, which reports a sequence far enough ahead as *behind*.
        // Comparing explicitly keeps a large gap classified as a future batch to buffer.
        match seq.cmp(&self.next_preemptive) {
            Ordering::Less => {
                return Err(PreemptiveError::Backtracking(seq, update_batch));
            }
            Ordering::Greater => {
                if self.staged.len() >= MAX_STAGED_BATCHES {
                    return Err(PreemptiveError::ReorderBufferFull {
                        seq,
                        awaiting: self.next_preemptive,
                        buffered: self.staged.len(),
                    });
                }

                self.staged.insert(seq, update_batch);
                metric_increment(REORDER_STAGED_COUNT_ID, Some(1));
                metric_store_count(REORDER_BUFFER_SIZE_ID, self.staged.len());

                return Ok(PreemptiveOutcome::Staged {
                    awaiting: self.next_preemptive,
                    buffered: self.staged.len(),
                });
            }
            Ordering::Equal => {}
        }

        self.execute_speculatively(application, update_batch);

        // This batch may have closed the gap in front of batches that arrived early.
        let mut drained = 0;
        while let Some(buffered) = self.staged.remove(&self.next_preemptive) {
            self.execute_speculatively(application, buffered);
            drained += 1;
        }

        if drained > 0 {
            metric_store_count(REORDER_BUFFER_SIZE_ID, self.staged.len());
        }

        Ok(PreemptiveOutcome::Executed { drained })
    }

    /// Speculatively execute one batch that is known to be the next in sequence, using the
    /// parallel + collision-resolution pipeline described on `handle_preemptive_update`.
    fn execute_speculatively(&mut self, application: &A, update_batch: UpdateBatch<Request<A, S>>) {
        let exec_start = Instant::now();

        let seq = update_batch.sequence_number();
        let batch_copy = update_batch.clone();
        let (_, updates) = update_batch.into_inner();
        let n = updates.len();

        // Enumerate updates into (position, info, op). Clone op so we can
        // re-execute colliders later without consuming the original.
        let indexed: Vec<(usize, _, Request<A, S>)> = updates
            .into_iter()
            .enumerate()
            .map(|(i, item)| {
                let (info, op) = item.into_inner();
                (i, info, op)
            })
            .collect();

        // Step 1: Parallel speculative execution.
        // Each request gets its own ParallelExecutionUnit that reads from:
        //   own local delta → upper accumulated cache → confirmed state.
        // All reads (including accumulated-cache fallthrough) are tracked as accesses.
        let confirmed_state = &self.confirmed_state;
        let accumulated_cache = &self.accumulated_cache;

        // Result: (position, accesses, local_cache, info, op_for_reexec, precomputed_reply)
        // We keep a clone of the op alongside the result so that colliders can be re-executed.
        let mut parallel_results: Vec<ParallelResultEntry<A, S>> = self.thread_pool.install(|| {
            indexed
                .into_par_iter()
                .map(|(pos, info, op)| {
                    let op_clone = op.clone();
                    let mut unit = ParallelExecutionUnit::new(confirmed_state, accumulated_cache);
                    let reply = application.speculatively_execute(&mut unit, op);
                    let (accesses, local_cache) = unit.complete();
                    (pos, accesses, local_cache, info, op_clone, reply)
                })
                .collect()
        });

        // Sort by position so we can iterate in batch order.
        parallel_results.sort_unstable_by_key(|(pos, ..)| *pos);

        // Step 2: Collision detection.
        let mut collision_state = CollisionState::default();
        for (pos, accesses, ..) in &parallel_results {
            progress_collision_state(&mut collision_state, *pos, accesses);
        }
        metric_store_count(
            SCALABLE_COLLISION_COUNT_ID,
            collision_state.collisions.len(),
        );
        metric_store_count(SCALABLE_OPS_PER_BATCH_ID, n);
        if let Some(rate) = (collision_state.collisions.len() * 1000).checked_div(n) {
            metric_store_count(SCALABLE_COLLISION_RATE_ID, rate);
        }

        // Step 3 & 4: Partition results into non-colliders and colliders (both in batch order
        // since parallel_results is already sorted). Consume parallel_results by value to avoid
        // any Clone requirement on UpdateInfo or Reply.
        let (non_colliders, colliders): (Vec<_>, Vec<_>) = parallel_results
            .into_iter()
            .partition(|(pos, ..)| !collision_state.collisions.contains(pos));

        let mut batch_delta = AccumulatedCache::default();
        // reply_holders: (position, UpdateInfo, reply) — built across both passes.
        let mut reply_holders: Vec<(usize, _, Reply<A, S>)> = Vec::with_capacity(n);

        // First pass: apply non-colliders in batch order, collect their replies.
        for (pos, _, local_cache, info, _, reply) in non_colliders {
            merge_delta_into(&mut batch_delta, &local_cache);
            reply_holders.push((pos, info, reply));
        }

        // Build the running merged view: upper accumulated cache + non-collider batch_delta.
        let mut current_view = self.accumulated_cache.clone();
        merge_delta_into(&mut current_view, &batch_delta);

        // Second pass: re-execute colliders sequentially in batch order.
        // Each collider sees all non-colliders' writes plus all earlier colliders' writes.
        for (pos, _, _, info, op_clone, _) in colliders {
            let mut caching_state = CachingState::new(&self.confirmed_state, &current_view);
            let reply = application.speculatively_execute(&mut caching_state, op_clone);
            let new_delta = caching_state.into_delta();
            merge_delta_into(&mut batch_delta, &new_delta);
            merge_delta_into(&mut current_view, &new_delta);
            reply_holders.push((pos, info, reply));
        }

        // Step 5: Sort by position and build the final ReplyBatch.
        reply_holders.sort_unstable_by_key(|(pos, ..)| *pos);
        let mut replies = ReplyBatch::new_with_cap(n);
        for (_, info, reply) in reply_holders {
            replies.add(info, reply);
        }

        // Merge batch_delta into the upper accumulated cache.
        merge_delta_into(&mut self.accumulated_cache, &batch_delta);
        self.next_preemptive = seq.next();
        self.pending.push_back(PendingCachedUpdate {
            batch: batch_copy,
            replies,
            delta: batch_delta,
            speculated_at: Instant::now(),
        });

        metric_duration(SCALABLE_PREEMPTIVE_EXECUTION_TIME_ID, exec_start.elapsed());
    }

    // -----------------------------------------------------------------------
    // All paths below are identical to CachingPreemptiveState
    // -----------------------------------------------------------------------

    /// Confirm the head pending update. If the batch was never speculated because it was
    /// still waiting in the reorder buffer, it is executed directly against the confirmed
    /// state instead, so the decision still lands and clients still get replies.
    pub(super) fn handle_update_confirmed(
        &mut self,
        application: &A,
        sequence_no: SeqNo,
    ) -> Result<ReplyBatch<Reply<A, S>>, ConfirmError> {
        if sequence_no != self.next_confirmed {
            return Err(ConfirmError::OutOfOrderConfirmation {
                expected: self.next_confirmed,
                received: sequence_no,
            });
        }

        // Resolved before mutating so the borrow of `pending` ends here.
        let route = match self.pending.front() {
            Some(p) if p.sequence_number() == sequence_no => ConfirmRoute::Pending,
            _ if self.staged.contains_key(&sequence_no) => ConfirmRoute::Staged,
            Some(p) => ConfirmRoute::Mismatch(p.sequence_number()),
            None => ConfirmRoute::Empty,
        };

        match route {
            ConfirmRoute::Pending => {}
            ConfirmRoute::Staged => {
                let batch = self
                    .staged
                    .remove(&sequence_no)
                    .expect("presence checked when choosing the route");

                metric_increment(SPECULATION_FALLBACK_COUNT_ID, Some(1));

                // Counted here rather than inside execute_directly, which consumes the
                // batch. This path still commits every operation in it, so it counts
                // towards throughput exactly like the speculated path below.
                metric_increment(OPERATIONS_EXECUTED_PER_SECOND_ID, Some(batch.len() as u64));

                // Anything already speculated raced ahead of a batch that never ran, so its
                // deltas are computed against a base state that is about to change. Return
                // those batches to the reorder buffer and rebuild speculation behind this one.
                self.discard_speculation();

                return Ok(self.execute_directly(application, batch));
            }
            ConfirmRoute::Mismatch(expected) => {
                return Err(ConfirmError::SeqMismatch {
                    expected,
                    received: sequence_no,
                });
            }
            ConfirmRoute::Empty => {
                return Err(ConfirmError::EmptyQueue {
                    received: sequence_no,
                });
            }
        }

        let update = self.pending.pop_front().unwrap();

        // Counted at confirmation rather than at speculation — see the note on the same
        // increment in `single_threaded_crud::pending_state`. It matters more here: the
        // scalable executor also re-executes colliding operations *within* a batch, and
        // counting each execution would inflate throughput by the collision rate.
        metric_increment(
            OPERATIONS_EXECUTED_PER_SECOND_ID,
            Some(update.batch.len() as u64),
        );

        apply_delta_to_state(&mut self.confirmed_state, &update.delta);
        self.accumulated_cache = rebuild_accumulated_cache(self.pending.iter().map(|p| &p.delta));
        self.next_confirmed = sequence_no.next();

        Ok(update.replies)
    }

    /// Execute a batch straight against the confirmed state, bypassing speculation, and
    /// resume speculating from whatever is already buffered behind it.
    fn execute_directly(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> ReplyBatch<Reply<A, S>> {
        let seq = update_batch.sequence_number();
        let replies = application.update_batch(&mut self.confirmed_state, update_batch);

        self.next_confirmed = seq.next();
        self.next_preemptive = seq.next();

        while let Some(buffered) = self.staged.remove(&self.next_preemptive) {
            self.execute_speculatively(application, buffered);
        }
        metric_store_count(REORDER_BUFFER_SIZE_ID, self.staged.len());

        replies
    }

    /// Throw away all speculative work, returning the underlying batches to the reorder
    /// buffer so no decision is lost — only the deltas and replies computed for them.
    fn discard_speculation(&mut self) {
        for update in self.pending.drain(..) {
            self.staged
                .insert(update.batch.sequence_number(), update.batch);
        }
        self.accumulated_cache = AccumulatedCache::default();
        self.next_preemptive = self.next_confirmed;
        metric_store_count(REORDER_BUFFER_SIZE_ID, self.staged.len());
    }

    pub(super) fn handle_confirmed_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> Result<ReplyBatch<Reply<A, S>>, ConfirmError> {
        if self.next_preemptive != self.next_confirmed {
            return Err(ConfirmError::PendingPreemptiveUpdates {
                confirmed_update_seq: update_batch.sequence_number(),
                next_preemptive: self.next_preemptive,
                next_confirmed: self.next_confirmed,
            });
        }

        let seq = update_batch.sequence_number();
        let replies = application.update_batch(&mut self.confirmed_state, update_batch);

        self.next_confirmed = seq.next();
        self.next_preemptive = seq.next();

        Ok(replies)
    }

    pub(super) fn handle_catch_up(
        &mut self,
        application: &A,
        batches: impl IntoIterator<Item = UpdateBatch<Request<A, S>>>,
    ) -> Vec<(SeqNo, ReplyBatch<Reply<A, S>>)> {
        self.pending.clear();
        self.staged.clear();
        self.accumulated_cache = AccumulatedCache::default();

        let mut results = Vec::new();
        for batch in batches {
            let seq = batch.sequence_number();
            let replies = application.update_batch(&mut self.confirmed_state, batch);
            self.next_confirmed = seq.next();
            self.next_preemptive = seq.next();
            results.push((seq, replies));
        }
        results
    }

    pub(super) fn install_confirmed_state(&mut self, seq: SeqNo, state: S) {
        self.confirmed_state = state;
        self.next_confirmed = seq.next();
        self.next_preemptive = seq.next();
        self.accumulated_cache = AccumulatedCache::default();
        self.pending.clear();
        self.staged.clear();
    }

    pub(super) fn backtrack(&mut self, backtrack_seq: SeqNo) -> Result<(), BacktrackError> {
        if backtrack_seq.cmp(&self.next_confirmed) == Ordering::Less {
            return Err(BacktrackError::BacktrackBelowConfirmed {
                backtrack_seq,
                next_confirmed: self.next_confirmed,
            });
        }

        self.pending
            .retain(|p| p.sequence_number().cmp(&backtrack_seq) == Ordering::Less);

        // Buffered batches at or above the backtrack point describe decisions that are
        // being re-decided, so the copies held here are stale.
        self.staged
            .retain(|seq, _| seq.cmp(&backtrack_seq) == Ordering::Less);

        self.next_preemptive = self
            .pending
            .back()
            .map(|p| p.sequence_number().next())
            .unwrap_or(self.next_confirmed);

        self.accumulated_cache = rebuild_accumulated_cache(self.pending.iter().map(|p| &p.delta));
        metric_store_count(REORDER_BUFFER_SIZE_ID, self.staged.len());

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Error types (mirrors single_threaded_crud)
// ---------------------------------------------------------------------------

/// Where the batch for an incoming confirmation lives.
enum ConfirmRoute {
    /// At the head of the pending queue, already speculated.
    Pending,
    /// Still in the reorder buffer, never speculated.
    Staged,
    /// The pending queue holds a different sequence number.
    Mismatch(SeqNo),
    /// Neither queue holds it.
    Empty,
}

#[derive(Error)]
pub(super) enum PreemptiveError<A, S>
where
    A: Application<S>,
{
    #[error("Backtracking required at seq {0:?}")]
    Backtracking(SeqNo, UpdateBatch<Request<A, S>>),
    #[error(
        "Reorder buffer full ({buffered} batches) while waiting for {awaiting:?}; \
         cannot buffer {seq:?}"
    )]
    ReorderBufferFull {
        seq: SeqNo,
        awaiting: SeqNo,
        buffered: usize,
    },
}

impl<A, S> Debug for PreemptiveError<A, S>
where
    A: Application<S>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PreemptiveError::Backtracking(seq, batch) => f
                .debug_tuple("Backtracking")
                .field(seq)
                .field(&format!("batch({} ops)", batch.len()))
                .finish(),
            PreemptiveError::ReorderBufferFull {
                seq,
                awaiting,
                buffered,
            } => f
                .debug_struct("ReorderBufferFull")
                .field("seq", seq)
                .field("awaiting", awaiting)
                .field("buffered", buffered)
                .finish(),
        }
    }
}

#[derive(Debug, Error)]
pub(super) enum ConfirmError {
    #[error("Seq mismatch: expected {expected:?}, received {received:?}")]
    SeqMismatch { expected: SeqNo, received: SeqNo },
    #[error("Pending queue is empty, received confirmation for seq {received:?}")]
    EmptyQueue { received: SeqNo },
    #[error("Confirmation for {received:?} arrived out of order, expected {expected:?}")]
    OutOfOrderConfirmation { expected: SeqNo, received: SeqNo },
    #[error(
        "Confirmed update ({confirmed_update_seq:?}) arrived while preemptive updates are pending \
         (next_preemptive={next_preemptive:?}, next_confirmed={next_confirmed:?})"
    )]
    PendingPreemptiveUpdates {
        confirmed_update_seq: SeqNo,
        next_preemptive: SeqNo,
        next_confirmed: SeqNo,
    },
}

#[derive(Debug, Error)]
pub(super) enum BacktrackError {
    #[error("Cannot backtrack to {backtrack_seq:?}: already confirmed through {next_confirmed:?}")]
    BacktrackBelowConfirmed {
        backtrack_seq: SeqNo,
        next_confirmed: SeqNo,
    },
}
