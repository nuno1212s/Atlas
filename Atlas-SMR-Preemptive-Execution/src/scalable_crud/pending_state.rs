use crate::metric::{SCALABLE_COLLISION_COUNT_ID, SCALABLE_PREEMPTIVE_EXECUTION_TIME_ID};
use crate::scalable_crud::execution_unit::{
    CollisionState, ParallelExecutionUnit, progress_collision_state,
};
use crate::single_threaded_crud::caching_state::{
    AccumulatedCache, CachingState, apply_delta_to_state, merge_delta_into,
    rebuild_accumulated_cache,
};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::execution::requests::{
    IncrementableUpdateBatch, ReplyBatch, UpdateBatch, UpdateInfo,
};
use atlas_metrics::metrics::{metric_duration, metric_store_count};
use atlas_smr_application::app::{Application, Reply, Request};
use atlas_smr_execution::crud_states::{Access, CRUDApplication, CRUDState};
use either::Either;
use rayon::ThreadPool;
use rayon::prelude::*;
use std::collections::VecDeque;
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
    confirmed_seq_no: SeqNo,
    preemptive_seq_no: SeqNo,
    accumulated_cache: AccumulatedCache,
    pending: VecDeque<PendingCachedUpdate<A, S>>,
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
            confirmed_seq_no: initial_state.0,
            preemptive_seq_no: initial_state.0,
            accumulated_cache: AccumulatedCache::default(),
            pending: VecDeque::new(),
            thread_pool,
        }
    }

    pub(super) fn confirmed_seq_no(&self) -> SeqNo {
        self.confirmed_seq_no
    }

    pub(super) fn preemptive_seq_no(&self) -> SeqNo {
        self.preemptive_seq_no
    }

    pub(super) fn pending_count(&self) -> usize {
        self.pending.len()
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
    ) -> Result<(), PreemptiveError<A, S>> {
        match update_batch.sequence_number().index(self.preemptive_seq_no) {
            // Left: incoming seq is older than current head — must backtrack.
            // Right(0): incoming seq == current head — re-execution of the same slot, also a backtrack.
            Either::Left(_) | Either::Right(0) => {
                return Err(PreemptiveError::Backtracking(
                    update_batch.sequence_number(),
                    update_batch,
                ));
            }
            Either::Right(1) => {}
            Either::Right(_) => {
                return Err(PreemptiveError::FutureRequest(
                    update_batch.sequence_number(),
                    self.preemptive_seq_no,
                ));
            }
        }

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
        self.preemptive_seq_no = seq;
        self.pending.push_back(PendingCachedUpdate {
            batch: batch_copy,
            replies,
            delta: batch_delta,
            speculated_at: Instant::now(),
        });

        metric_duration(SCALABLE_PREEMPTIVE_EXECUTION_TIME_ID, exec_start.elapsed());

        Ok(())
    }

    // -----------------------------------------------------------------------
    // All paths below are identical to CachingPreemptiveState
    // -----------------------------------------------------------------------

    pub(super) fn handle_update_confirmed(
        &mut self,
        sequence_no: SeqNo,
    ) -> Result<ReplyBatch<Reply<A, S>>, ConfirmError> {
        match self.pending.front() {
            Some(p) if p.batch.sequence_number() == sequence_no => {}
            Some(p) => {
                return Err(ConfirmError::SeqMismatch {
                    expected: p.batch.sequence_number(),
                    received: sequence_no,
                });
            }
            None => {
                return Err(ConfirmError::EmptyQueue {
                    received: sequence_no,
                });
            }
        }

        let update = self.pending.pop_front().unwrap();

        apply_delta_to_state(&mut self.confirmed_state, &update.delta);
        self.accumulated_cache = rebuild_accumulated_cache(self.pending.iter().map(|p| &p.delta));
        self.confirmed_seq_no = sequence_no;

        Ok(update.replies)
    }

    pub(super) fn handle_confirmed_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> Result<ReplyBatch<Reply<A, S>>, ConfirmError> {
        if self.preemptive_seq_no != self.confirmed_seq_no {
            return Err(ConfirmError::PendingPreemptiveUpdates {
                confirmed_update_seq: update_batch.sequence_number(),
                preemptive_seq: self.preemptive_seq_no,
                confirmed_seq: self.confirmed_seq_no,
            });
        }

        let seq = update_batch.sequence_number();
        let replies = application.update_batch(&mut self.confirmed_state, update_batch);

        self.confirmed_seq_no = seq;
        self.preemptive_seq_no = seq;

        Ok(replies)
    }

    pub(super) fn handle_catch_up(
        &mut self,
        application: &A,
        batches: impl IntoIterator<Item = UpdateBatch<Request<A, S>>>,
    ) -> Vec<(SeqNo, ReplyBatch<Reply<A, S>>)> {
        self.pending.clear();
        self.accumulated_cache = AccumulatedCache::default();

        let mut results = Vec::new();
        for batch in batches {
            let seq = batch.sequence_number();
            let replies = application.update_batch(&mut self.confirmed_state, batch);
            self.confirmed_seq_no = seq;
            self.preemptive_seq_no = seq;
            results.push((seq, replies));
        }
        results
    }

    pub(super) fn install_confirmed_state(&mut self, seq: SeqNo, state: S) {
        self.confirmed_state = state;
        self.confirmed_seq_no = seq;
        self.preemptive_seq_no = seq;
        self.accumulated_cache = AccumulatedCache::default();
        self.pending.clear();
    }

    pub(super) fn backtrack(&mut self, backtrack_seq: SeqNo) -> Result<(), BacktrackError> {
        if backtrack_seq <= self.confirmed_seq_no {
            return Err(BacktrackError::BacktrackToConfirmedOrBelow {
                backtrack_seq,
                confirmed_seq: self.confirmed_seq_no,
            });
        }

        self.pending.retain(|p| p.sequence_number() < backtrack_seq);

        self.preemptive_seq_no = self
            .pending
            .back()
            .map(|p| p.sequence_number())
            .unwrap_or(self.confirmed_seq_no);

        self.accumulated_cache = rebuild_accumulated_cache(self.pending.iter().map(|p| &p.delta));

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Error types (mirrors single_threaded_crud)
// ---------------------------------------------------------------------------

#[derive(Error)]
pub(super) enum PreemptiveError<A, S>
where
    A: Application<S>,
{
    #[error("Backtracking required at seq {0:?}")]
    Backtracking(SeqNo, UpdateBatch<Request<A, S>>),
    #[error("Future request: received seq {0:?}, current preemptive seq {1:?}")]
    FutureRequest(SeqNo, SeqNo),
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
            PreemptiveError::FutureRequest(seq, current) => f
                .debug_tuple("FutureRequest")
                .field(seq)
                .field(current)
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
    #[error(
        "Confirmed update ({confirmed_update_seq:?}) arrived while preemptive updates are pending \
         (preemptive_seq={preemptive_seq:?}, confirmed_seq={confirmed_seq:?})"
    )]
    PendingPreemptiveUpdates {
        confirmed_update_seq: SeqNo,
        preemptive_seq: SeqNo,
        confirmed_seq: SeqNo,
    },
}

#[derive(Debug, Error)]
pub(super) enum BacktrackError {
    #[error(
        "Cannot backtrack to {backtrack_seq:?}: must be strictly above confirmed seq {confirmed_seq:?}"
    )]
    BacktrackToConfirmedOrBelow {
        backtrack_seq: SeqNo,
        confirmed_seq: SeqNo,
    },
}
