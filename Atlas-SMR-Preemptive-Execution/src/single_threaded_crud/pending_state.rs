use crate::metric::{
    CACHE_BACKTRACK_COUNT_ID, CACHE_CONFIRM_APPLICATION_TIME_ID, CACHE_DELTA_SIZE_ID,
    CACHE_OPS_PER_BATCH_ID, CACHE_PENDING_QUEUE_SIZE_ID, CACHE_PREEMPTIVE_EXECUTION_TIME_ID,
    CACHE_REBUILD_TIME_ID, CACHE_SPECULATION_TO_CONFIRM_LATENCY_ID, REORDER_BUFFER_SIZE_ID,
    REORDER_STAGED_COUNT_ID, SPECULATION_FALLBACK_COUNT_ID,
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
use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::fmt::{Debug, Formatter};
use std::time::Instant;
use thiserror::Error;

/// Upper bound on the number of out-of-order batches held in the reorder buffer.
///
/// The ordering protocol runs several consensus instances concurrently, so batches reach
/// the executor out of sequence — but the decision log sends each batch exactly once over a
/// blocking channel, so they are never dropped and the gaps always close. Measured reorder
/// displacement on a 4-replica PBFT run is at most 15 batches, so this bound leaves more
/// than two orders of magnitude of headroom: hitting it means a batch was genuinely lost,
/// not merely late.
pub(crate) const MAX_STAGED_BATCHES: usize = 4096;

pub(crate) struct PendingCachedUpdate<A, S>
where
    A: Application<S>,
{
    pub(crate) batch: UpdateBatch<Request<A, S>>,
    pub(crate) replies: ReplyBatch<Reply<A, S>>,
    pub(crate) delta: AccumulatedCache,
    /// Timestamp captured at the start of speculative execution.
    pub(crate) speculated_at: Instant,
}

impl<A, S> Orderable for PendingCachedUpdate<A, S>
where
    A: Application<S>,
{
    fn sequence_number(&self) -> SeqNo {
        self.batch.sequence_number()
    }
}

/// What happened to a batch handed to [`CachingPreemptiveState::handle_preemptive_update`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PreemptiveOutcome {
    /// The batch was executed speculatively. `drained` counts additional batches that were
    /// released from the reorder buffer behind it because this batch closed their gap.
    Executed { drained: usize },
    /// The batch arrived ahead of its turn and is held in the reorder buffer until the
    /// executor reaches `awaiting`.
    Staged { awaiting: SeqNo, buffered: usize },
}

/// Core state machine for cache-based sequential preemptive execution.
///
/// The real (confirmed) state is only mutated on confirmation.
/// Preemptive updates write into per-update deltas kept in an in-memory pending queue.
/// The accumulated cache (merge of all pending deltas) answers reads that fall through
/// from the current update's local delta.
///
/// Sequence tracking is expressed as the *next expected* sequence number rather than the
/// last applied one. That distinction matters: `SeqNo::ZERO` is both the initial value and
/// the first sequence number consensus ever assigns, so a "last applied" field cannot tell
/// "nothing has run yet" apart from "sequence 0 has run" — and the executor would reject
/// the very first batch of every run.
pub(crate) struct CachingPreemptiveState<S, A>
where
    A: CRUDApplication<S>,
    S: CRUDState + Sync,
{
    confirmed_state: S,
    /// Sequence number the next confirmation is expected to carry.
    next_confirmed: SeqNo,
    /// Sequence number the next preemptive update is expected to carry.
    next_preemptive: SeqNo,
    /// Merge of all pending deltas; checked on reads before the real state.
    accumulated_cache: AccumulatedCache,
    /// FIFO queue of pending updates awaiting confirmation.
    pending: VecDeque<PendingCachedUpdate<A, S>>,
    /// Batches that arrived ahead of `next_preemptive`, keyed by sequence number and held
    /// until the gap in front of them closes. Draining is strictly in order, so a batch
    /// only ever leaves this buffer once every sequence number below it has been executed.
    staged: BTreeMap<SeqNo, UpdateBatch<Request<A, S>>>,
}

impl<S, A> CachingPreemptiveState<S, A>
where
    A: CRUDApplication<S>,
    S: CRUDState + Sync,
{
    pub(crate) fn new(initial_state: (SeqNo, S)) -> Self {
        Self {
            confirmed_state: initial_state.1,
            next_confirmed: initial_state.0,
            next_preemptive: initial_state.0,
            accumulated_cache: AccumulatedCache::default(),
            pending: VecDeque::new(),
            staged: BTreeMap::new(),
        }
    }

    /// The sequence number the next confirmation is expected to carry.
    pub(crate) fn next_confirmed_seq(&self) -> SeqNo {
        self.next_confirmed
    }

    /// The sequence number the next preemptive update is expected to carry.
    pub(crate) fn next_preemptive_seq(&self) -> SeqNo {
        self.next_preemptive
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn staged_count(&self) -> usize {
        self.staged.len()
    }

    pub(crate) fn pending_front_seq(&self) -> Option<SeqNo> {
        self.pending.front().map(|p| p.sequence_number())
    }

    pub(crate) fn confirmed_state(&self) -> &S {
        &self.confirmed_state
    }

    /// Preemptively execute `update_batch` through a `CachingState` proxy.
    ///
    /// Writes go into a per-update delta; the real state is never touched.
    /// The computed replies are stored in the pending queue until confirmation.
    ///
    /// A batch that arrives ahead of its turn is buffered rather than dropped, and is
    /// replayed automatically once the batches in front of it arrive.
    pub(crate) fn handle_preemptive_update(
        &mut self,
        application: &A,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> Result<PreemptiveOutcome, PreemptiveError<A, S>> {
        let seq = update_batch.sequence_number();

        // `SeqNo` compares by raw value under `Ord` (its `PartialOrd` instead goes through
        // the wrap-aware `index`, which reports a sequence far enough ahead as *behind*).
        // Comparing explicitly keeps a large gap classified as a future batch to buffer
        // rather than as a backtrack.
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

    /// Speculatively execute one batch that is known to be the next in sequence.
    fn execute_speculatively(&mut self, application: &A, update_batch: UpdateBatch<Request<A, S>>) {
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
        self.next_preemptive = seq.next();
        self.pending.push_back(PendingCachedUpdate {
            batch: batch_copy,
            replies,
            delta,
            speculated_at: Instant::now(),
        });
    }

    /// Confirm the head pending update (must be at `sequence_no`).
    ///
    /// Applies its delta to the real state, rebuilds the accumulated cache from the
    /// remaining pending deltas, and returns the pre-computed replies.
    ///
    /// If the batch was never speculated because it was still waiting in the reorder
    /// buffer, it is executed directly against the confirmed state instead: the decision
    /// still lands and clients still get replies, at the cost of the speculative work.
    pub(crate) fn handle_update_confirmed(
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

                // Anything already speculated raced ahead of a batch that never ran, so its
                // deltas are computed against a base state that is about to change. Return
                // those batches to the reorder buffer — their decisions are still owed
                // replies — and rebuild speculation from scratch behind this one.
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

        // Batches buffered behind this one can speculate again now that the gap is closed.
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

    /// Execute a directly-finalized batch (no prior preemptive execution for this seq).
    ///
    /// Requires no pending speculative work: `next_preemptive == next_confirmed`.
    pub(crate) fn handle_confirmed_update(
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

    /// Apply catch-up batches directly to the confirmed state and return replies.
    ///
    /// Clears all pending speculative work, the reorder buffer and the accumulated cache.
    pub(crate) fn handle_catch_up(
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

    /// Install a confirmed state snapshot (state transfer).
    /// Discards all pending speculative work and any buffered batches, which belong to a
    /// sequence range the installed state has already subsumed.
    pub(crate) fn install_confirmed_state(&mut self, seq: SeqNo, state: S) {
        self.confirmed_state = state;
        self.next_confirmed = seq.next();
        self.next_preemptive = seq.next();
        self.accumulated_cache = AccumulatedCache::default();
        self.pending.clear();
        self.staged.clear();
    }

    /// Discard all pending updates at seq >= `backtrack_seq` and rebuild the cache.
    ///
    /// Updates with seq < `backtrack_seq` are kept — their cache contributions remain valid.
    /// The confirmed state is unchanged. No re-execution is required for kept batches.
    pub(crate) fn backtrack(&mut self, backtrack_seq: SeqNo) -> Result<(), BacktrackError> {
        if backtrack_seq.cmp(&self.next_confirmed) == Ordering::Less {
            return Err(BacktrackError::BacktrackBelowConfirmed {
                backtrack_seq,
                next_confirmed: self.next_confirmed,
            });
        }

        metric_increment(CACHE_BACKTRACK_COUNT_ID, Some(1));

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

        let rebuild_start = Instant::now();
        self.accumulated_cache = rebuild_accumulated_cache(self.pending.iter().map(|p| &p.delta));
        metric_duration(CACHE_REBUILD_TIME_ID, rebuild_start.elapsed());
        metric_store_count(REORDER_BUFFER_SIZE_ID, self.staged.len());

        Ok(())
    }
}

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

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Error)]
pub(crate) enum PreemptiveError<A, S>
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
pub(crate) enum ConfirmError {
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
pub(crate) enum BacktrackError {
    #[error("Cannot backtrack to {backtrack_seq:?}: already confirmed through {next_confirmed:?}")]
    BacktrackBelowConfirmed {
        backtrack_seq: SeqNo,
        next_confirmed: SeqNo,
    },
}
