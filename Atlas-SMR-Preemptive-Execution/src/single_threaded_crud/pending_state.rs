use crate::metric::{
    CACHE_BACKTRACK_COUNT_ID, CACHE_CONFIRM_APPLICATION_TIME_ID, CACHE_DELTA_SIZE_ID,
    CACHE_OPS_PER_BATCH_ID, CACHE_PENDING_QUEUE_SIZE_ID, CACHE_PREEMPTIVE_EXECUTION_TIME_ID,
    CACHE_REBUILD_TIME_ID, CACHE_SPECULATION_TO_CONFIRM_LATENCY_ID,
};
use crate::single_threaded_crud::caching_state::{
    AccumulatedCache, CachingState, apply_delta_to_state, merge_delta_into,
    rebuild_accumulated_cache,
};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::execution::requests::{IncrementableUpdateBatch, ReplyBatch, UpdateBatch};
use atlas_metrics::metrics::{metric_duration, metric_increment, metric_store_count};
use atlas_smr_application::app::{Application, Reply, Request};
use atlas_smr_execution::crud_states::{CRUDApplication, CRUDState};
use either::Either;
use std::collections::VecDeque;
use std::fmt::{Debug, Formatter};
use std::time::Instant;
use thiserror::Error;

pub(super) struct PendingCachedUpdate<A, S>
where
    A: Application<S>,
{
    pub(super) batch: UpdateBatch<Request<A, S>>,
    pub(super) replies: ReplyBatch<Reply<A, S>>,
    pub(super) delta: AccumulatedCache,
    /// Timestamp captured at the start of speculative execution.
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

/// Core state machine for cache-based sequential preemptive execution.
///
/// The real (confirmed) state is only mutated on confirmation.
/// Preemptive updates write into per-update deltas kept in an in-memory pending queue.
/// The accumulated cache (merge of all pending deltas) answers reads that fall through
/// from the current update's local delta.
pub(super) struct CachingPreemptiveState<S, A>
where
    A: CRUDApplication<S>,
    S: CRUDState + Sync,
{
    confirmed_state: S,
    confirmed_seq_no: SeqNo,
    /// Sequence number of the last preemptively executed update.
    preemptive_seq_no: SeqNo,
    /// Merge of all pending deltas; checked on reads before the real state.
    accumulated_cache: AccumulatedCache,
    /// FIFO queue of pending updates awaiting confirmation.
    pending: VecDeque<PendingCachedUpdate<A, S>>,
}

impl<S, A> CachingPreemptiveState<S, A>
where
    A: CRUDApplication<S>,
    S: CRUDState + Sync,
{
    pub(super) fn new(initial_state: (SeqNo, S)) -> Self {
        Self {
            confirmed_state: initial_state.1,
            confirmed_seq_no: initial_state.0,
            preemptive_seq_no: initial_state.0,
            accumulated_cache: AccumulatedCache::default(),
            pending: VecDeque::new(),
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

    /// Preemptively execute `update_batch` through a `CachingState` proxy.
    ///
    /// Writes go into a per-update delta; the real state is never touched.
    /// The computed replies are stored in the pending queue until confirmation.
    pub(super) fn handle_preemptive_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> Result<(), PreemptiveError<A, S>> {
        match update_batch.sequence_number().index(self.preemptive_seq_no) {
            Either::Left(_) => {
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

        let seq = update_batch.sequence_number();
        let batch_copy = update_batch.clone();
        let (_, updates) = update_batch.into_inner();

        metric_store_count(CACHE_OPS_PER_BATCH_ID, updates.len());

        let mut caching_state = CachingState::new(&self.confirmed_state, &self.accumulated_cache);
        let mut replies = ReplyBatch::new_with_cap(updates.len());

        let exec_start = Instant::now();
        for item in updates {
            let (info, op) = item.into_inner();
            let reply = application.speculatively_execute(&mut caching_state, op);
            replies.add(info, reply);
        }
        metric_duration(CACHE_PREEMPTIVE_EXECUTION_TIME_ID, exec_start.elapsed());

        let delta = caching_state.into_delta();
        merge_delta_into(&mut self.accumulated_cache, &delta);
        self.preemptive_seq_no = seq;
        self.pending.push_back(PendingCachedUpdate {
            batch: batch_copy,
            replies,
            delta,
            speculated_at: Instant::now(),
        });

        Ok(())
    }

    /// Confirm the head pending update (must be at `sequence_no`).
    ///
    /// Applies its delta to the real state, rebuilds the accumulated cache from the
    /// remaining pending deltas, and returns the pre-computed replies.
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

        metric_duration(
            CACHE_SPECULATION_TO_CONFIRM_LATENCY_ID,
            update.speculated_at.elapsed(),
        );
        metric_store_count(CACHE_PENDING_QUEUE_SIZE_ID, self.pending.len() + 1);

        let delta_size: usize = update.delta.values().map(|col| col.len()).sum();
        metric_store_count(CACHE_DELTA_SIZE_ID, delta_size);

        let confirm_start = Instant::now();
        apply_delta_to_state(&mut self.confirmed_state, &update.delta);
        metric_duration(CACHE_CONFIRM_APPLICATION_TIME_ID, confirm_start.elapsed());

        let rebuild_start = Instant::now();
        self.accumulated_cache = rebuild_accumulated_cache(self.pending.iter().map(|p| &p.delta));
        metric_duration(CACHE_REBUILD_TIME_ID, rebuild_start.elapsed());

        self.confirmed_seq_no = sequence_no;

        Ok(update.replies)
    }

    /// Execute a directly-finalized batch (no prior preemptive execution for this seq).
    ///
    /// Requires no pending speculative work: `preemptive_seq_no == confirmed_seq_no`.
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

    /// Apply catch-up batches directly to the confirmed state and return replies.
    ///
    /// Clears all pending speculative work and the accumulated cache.
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

    /// Install a confirmed state snapshot (state transfer).
    /// Discards all pending speculative work.
    pub(super) fn install_confirmed_state(&mut self, seq: SeqNo, state: S) {
        self.confirmed_state = state;
        self.confirmed_seq_no = seq;
        self.preemptive_seq_no = seq;
        self.accumulated_cache = AccumulatedCache::default();
        self.pending.clear();
    }

    /// Discard all pending updates at seq >= `backtrack_seq` and rebuild the cache.
    ///
    /// Updates with seq < `backtrack_seq` are kept — their cache contributions remain valid.
    /// The confirmed state is unchanged. No re-execution is required for kept batches.
    pub(super) fn backtrack(&mut self, backtrack_seq: SeqNo) -> Result<(), BacktrackError> {
        if backtrack_seq <= self.confirmed_seq_no {
            return Err(BacktrackError::BacktrackToConfirmedOrBelow {
                backtrack_seq,
                confirmed_seq: self.confirmed_seq_no,
            });
        }

        metric_increment(CACHE_BACKTRACK_COUNT_ID, Some(1));

        self.pending.retain(|p| p.sequence_number() < backtrack_seq);

        self.preemptive_seq_no = self
            .pending
            .back()
            .map(|p| p.sequence_number())
            .unwrap_or(self.confirmed_seq_no);

        let rebuild_start = Instant::now();
        self.accumulated_cache = rebuild_accumulated_cache(self.pending.iter().map(|p| &p.delta));
        metric_duration(CACHE_REBUILD_TIME_ID, rebuild_start.elapsed());

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Error types
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
