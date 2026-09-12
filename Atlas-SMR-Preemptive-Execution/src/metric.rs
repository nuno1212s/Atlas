use atlas_metrics::metrics::{MetricKind, metric_duration, metric_store_count};
use atlas_metrics::{MetricLevel, MetricRegistry};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Dual-state executor metrics (800-808)
// ---------------------------------------------------------------------------

/// Time from confirmed-update enqueue at the confirmed worker to actual execution.
pub const CONFIRMED_WORKER_LATENCY: &str = "CONFIRMED_WORKER_LATENCY";
pub const CONFIRMED_WORKER_LATENCY_ID: usize = 800;

/// Time for the confirmed worker to re-execute a batch on the authoritative state.
pub const CONFIRM_EXECUTION_TIME: &str = "CONFIRM_EXECUTION_TIME";
pub const CONFIRM_EXECUTION_TIME_ID: usize = 801;

/// Time for the preemptive worker to execute a batch speculatively.
pub const DS_PREEMPTIVE_EXECUTION_TIME: &str = "DS_PREEMPTIVE_EXECUTION_TIME";
pub const DS_PREEMPTIVE_EXECUTION_TIME_ID: usize = 804;

/// Time from when a batch was speculatively executed to when consensus confirms it.
pub const DS_SPECULATION_TO_CONFIRM_LATENCY: &str = "DS_SPECULATION_TO_CONFIRM_LATENCY";
pub const DS_SPECULATION_TO_CONFIRM_LATENCY_ID: usize = 805;

/// Number of backtrack events in the dual-state executor.
pub const DS_BACKTRACK_COUNT: &str = "DS_BACKTRACK_COUNT";
pub const DS_BACKTRACK_COUNT_ID: usize = 806;

/// Number of operations per speculatively executed batch (dual-state).
pub const DS_OPS_PER_BATCH: &str = "DS_OPS_PER_BATCH";
pub const DS_OPS_PER_BATCH_ID: usize = 807;

// ---------------------------------------------------------------------------
// Cache-based executor metrics (802-803, 809-816)
// ---------------------------------------------------------------------------

/// Time to speculatively execute a batch through the CachingState proxy.
pub const CACHE_PREEMPTIVE_EXECUTION_TIME: &str = "CACHE_PREEMPTIVE_EXECUTION_TIME";
pub const CACHE_PREEMPTIVE_EXECUTION_TIME_ID: usize = 802;

/// Time to apply a pre-computed delta to the confirmed state on confirmation.
pub const CACHE_CONFIRM_APPLICATION_TIME: &str = "CACHE_CONFIRM_APPLICATION_TIME";
pub const CACHE_CONFIRM_APPLICATION_TIME_ID: usize = 803;

/// Time from when a batch was speculatively executed to when consensus confirms it.
pub const CACHE_SPECULATION_TO_CONFIRM_LATENCY: &str = "CACHE_SPECULATION_TO_CONFIRM_LATENCY";
pub const CACHE_SPECULATION_TO_CONFIRM_LATENCY_ID: usize = 809;

/// Number of pending updates in the queue at the time a confirmation arrives.
pub const CACHE_PENDING_QUEUE_SIZE: &str = "CACHE_PENDING_QUEUE_SIZE";
pub const CACHE_PENDING_QUEUE_SIZE_ID: usize = 810;

/// Number of backtrack events in the cache-based executor.
pub const CACHE_BACKTRACK_COUNT: &str = "CACHE_BACKTRACK_COUNT";
pub const CACHE_BACKTRACK_COUNT_ID: usize = 811;

/// Number of key-value entries in the confirmed delta applied per batch.
pub const CACHE_DELTA_SIZE: &str = "CACHE_DELTA_SIZE";
pub const CACHE_DELTA_SIZE_ID: usize = 812;

/// Number of operations per speculatively executed batch (cache-based).
pub const CACHE_OPS_PER_BATCH: &str = "CACHE_OPS_PER_BATCH";
pub const CACHE_OPS_PER_BATCH_ID: usize = 813;

/// Time for unordered (read-only) execution via the rayon thread pool.
pub const CACHE_UNORDERED_EXECUTION_TIME: &str = "CACHE_UNORDERED_EXECUTION_TIME";
pub const CACHE_UNORDERED_EXECUTION_TIME_ID: usize = 814;

/// Time to rebuild the accumulated cache after a confirmation or backtrack.
pub const CACHE_REBUILD_TIME: &str = "CACHE_REBUILD_TIME";
pub const CACHE_REBUILD_TIME_ID: usize = 815;

/// Time from when a preemptive update was enqueued to when execution starts.
pub const CACHE_ENQUEUE_TO_EXECUTE_LATENCY: &str = "CACHE_ENQUEUE_TO_EXECUTE_LATENCY";
pub const CACHE_ENQUEUE_TO_EXECUTE_LATENCY_ID: usize = 816;

// ---------------------------------------------------------------------------
// Scalable CRUD executor metrics (817-820)
// ---------------------------  ------------------------------------------------

/// Total wall-clock time for one parallel+collision+reexec cycle in the scalable CRUD executor.
pub const SCALABLE_PREEMPTIVE_EXECUTION_TIME: &str = "SCALABLE_PREEMPTIVE_EXECUTION_TIME";
pub const SCALABLE_PREEMPTIVE_EXECUTION_TIME_ID: usize = 817;

/// Number of colliding operations per batch in the scalable CRUD executor.
pub const SCALABLE_COLLISION_COUNT: &str = "SCALABLE_COLLISION_COUNT";
pub const SCALABLE_COLLISION_COUNT_ID: usize = 818;

/// Per-batch collision rate in the scalable CRUD executor, stored as permille (×1000).
/// A value of 500 means 50.0% of requests in that batch experienced a key collision.
/// Divide by 10 to get a percentage, or by 1000 to get a [0.0, 1.0] fraction.
pub const SCALABLE_COLLISION_RATE: &str = "SCALABLE_COLLISION_RATE";
pub const SCALABLE_COLLISION_RATE_ID: usize = 819;

/// Number of operations per speculatively executed batch (scalable CRUD executor).
pub const SCALABLE_OPS_PER_BATCH: &str = "SCALABLE_OPS_PER_BATCH";
pub const SCALABLE_OPS_PER_BATCH_ID: usize = 820;

/// Time from when a batch was speculatively executed to when consensus confirms it
/// (scalable CRUD executor). The scalable executor captured this timestamp from the start
/// but never recorded it, which left it as the one variant whose reply-hold time could not
/// be compared against the other two.
pub const SCALABLE_SPECULATION_TO_CONFIRM_LATENCY: &str = "SCALABLE_SPECULATION_TO_CONFIRM_LATENCY";
pub const SCALABLE_SPECULATION_TO_CONFIRM_LATENCY_ID: usize = 826;

/// Time to apply a pre-computed delta to the confirmed state on confirmation
/// (scalable CRUD executor). Counterpart of `CACHE_CONFIRM_APPLICATION_TIME`, and the
/// other half of what the confirmation costs once speculation has paid off.
pub const SCALABLE_CONFIRM_APPLICATION_TIME: &str = "SCALABLE_CONFIRM_APPLICATION_TIME";
pub const SCALABLE_CONFIRM_APPLICATION_TIME_ID: usize = 827;

// ---------------------------------------------------------------------------
// Reorder buffer metrics (821-823)
// ---------------------------------------------------------------------------

/// Number of preemptive batches currently held in the reorder buffer, i.e. batches that
/// arrived ahead of the sequence number the executor is waiting for. Consensus delivers
/// batches out of order, so a non-zero value here is normal; a value that stops falling
/// means a batch is genuinely missing rather than merely late.
pub const REORDER_BUFFER_SIZE: &str = "REORDER_BUFFER_SIZE";
pub const REORDER_BUFFER_SIZE_ID: usize = 821;

/// Number of preemptive batches that arrived out of order and had to be buffered.
pub const REORDER_STAGED_COUNT: &str = "REORDER_STAGED_COUNT";
pub const REORDER_STAGED_COUNT_ID: usize = 822;

/// Number of times speculation was abandoned and the executor fell back to executing a
/// batch directly against the confirmed state. Expected to be zero in a healthy run.
pub const SPECULATION_FALLBACK_COUNT: &str = "SPECULATION_FALLBACK_COUNT";
pub const SPECULATION_FALLBACK_COUNT_ID: usize = 823;

// ---------------------------------------------------------------------------
// Confirmation path, shared by all three preemptive executors (828-831)
// ---------------------------------------------------------------------------
//
// Every metric above measures one variant, which is what makes them useless for the
// question the benchmark actually asks: *did speculation move work off the critical
// path?* These four carry the same name and the same meaning in all three, so a
// crud_perf comparison reads them as one series instead of three.
//
// They all measure spans that begin when the ordering protocol hands the executor a
// commit -- `queue_update_finalized` for a batch that was speculated, `queue_update`
// for one that was not. Both now stamp an `Instant` at that moment.
//
// The counterpart to the `*_SPECULATION_TO_CONFIRM_LATENCY` family, and the reason both
// directions are needed: that one is the reply waiting for consensus (speculation won
// the race), these are consensus waiting for the reply (it did not).

/// Time from the commit being queued at the executor to the executor dequeuing it.
/// Pure queueing on the confirmation path -- the executor thread was busy elsewhere,
/// most likely speculating. The confirmation-side mirror of
/// `CACHE_ENQUEUE_TO_EXECUTE_LATENCY`.
pub const CONFIRM_ENQUEUE_TO_APPLY_LATENCY: &str = "CONFIRM_ENQUEUE_TO_APPLY_LATENCY";
pub const CONFIRM_ENQUEUE_TO_APPLY_LATENCY_ID: usize = 828;

/// Time from the commit being queued at the executor to the replies being ready to
/// dispatch. The whole post-commit critical path, and the headline preemptive-vs-baseline
/// number: for the baseline this is queueing plus a full batch execution, for a preemptive
/// executor whose speculation hit it is queueing plus a delta apply.
///
/// Directly comparable to `EXECUTION_LATENCY + EXECUTION_TIME_TAKEN` in
/// `atlas-smr-execution`, which spans the same two points for the baseline executor.
pub const CONFIRM_TO_REPLY_TIME: &str = "CONFIRM_TO_REPLY_TIME";
pub const CONFIRM_TO_REPLY_TIME_ID: usize = 829;

/// `CONFIRM_TO_REPLY_TIME`, restricted to confirmations whose replies were *not* already
/// computed: consensus committed a batch speculation had not reached, so the execution
/// happened inline on the confirmation path.
///
/// `SPECULATION_FALLBACK_COUNT` says how often that happens; this says what it cost. The
/// gap between this and `CONFIRM_TO_REPLY_TIME` is what a speculation hit is worth.
pub const CONFIRM_BLOCKED_ON_EXEC_TIME: &str = "CONFIRM_BLOCKED_ON_EXEC_TIME";
pub const CONFIRM_BLOCKED_ON_EXEC_TIME_ID: usize = 830;

/// Fraction of confirmations served from pre-computed replies, as permille (x1000), so a
/// value of 1000 means every batch committed in that window had already been speculated.
/// Divide by 10 for a percentage. Permille rather than a fraction because `Count` averages
/// integers; the convention matches `SCALABLE_COLLISION_RATE`.
///
/// This is the run's sanity check. Speculation engages through Rust specialization, which
/// fails silently -- a replica that has quietly fallen back to post-commit execution still
/// builds, runs and produces plausible numbers. A hit rate pinned at 0 says the comparison
/// is measuring the baseline against itself.
pub const SPECULATION_HIT_RATE: &str = "SPECULATION_HIT_RATE";
pub const SPECULATION_HIT_RATE_ID: usize = 831;

/// Records the confirmation-path metrics shared by all three preemptive executors.
///
/// `confirmed_at` is the instant the ordering protocol queued the commit; `speculated` says
/// whether the replies for it had already been computed. Called once per confirmed batch,
/// immediately before the replies are handed to the replier.
pub(crate) fn record_confirmation(confirmed_at: Instant, speculated: bool) {
    let elapsed = confirmed_at.elapsed();

    metric_duration(CONFIRM_TO_REPLY_TIME_ID, elapsed);
    metric_store_count(SPECULATION_HIT_RATE_ID, if speculated { 1000 } else { 0 });

    if !speculated {
        metric_duration(CONFIRM_BLOCKED_ON_EXEC_TIME_ID, elapsed);
    }
}

// ---------------------------------------------------------------------------
// Shared execution throughput (824-825)
// ---------------------------------------------------------------------------
//
// These two deliberately carry the *same names* as their counterparts in
// `atlas-smr-execution` (IDs 804/805 there). Only one execution crate is ever linked
// into a binary, so the names cannot collide at runtime — and sharing them is the
// point: `OPERATIONS_EXECUTED_PER_SECOND` is the headline throughput series every
// suite dashboard plots, so a preemptive executor that did not emit it left the panel
// blank and made crud_perf incomparable with microbenchmarks-async.
//
// Counted on the **confirmation** path, never on the speculative one. A speculatively
// executed batch may be discarded and re-executed after a backtrack, so counting
// speculation would inflate throughput by exactly the work that was thrown away, and
// the number would no longer mean what it means for the baseline executor: operations
// whose results actually reached a client.

/// Ordered operations whose execution has been confirmed by consensus, counted per
/// one-second collection window. Same name and meaning as
/// `atlas_smr_execution::metric::OPERATIONS_EXECUTED_PER_SECOND`.
pub const OPERATIONS_EXECUTED_PER_SECOND: &str = "OPERATIONS_EXECUTED_PER_SECOND";
pub const OPERATIONS_EXECUTED_PER_SECOND_ID: usize = 824;

/// Unordered (read-only) operations executed, counted per one-second collection
/// window. Same name and meaning as `atlas_smr_execution::metric::UNORDERED_OPS_PER_SECOND`.
pub const UNORDERED_OPS_PER_SECOND: &str = "UNORDERED_OPERATIONS_EXECUTED_PER_SECOND";
pub const UNORDERED_OPS_PER_SECOND_ID: usize = 825;

pub fn metrics() -> Vec<MetricRegistry> {
    vec![
        (
            CONFIRMED_WORKER_LATENCY_ID,
            CONFIRMED_WORKER_LATENCY.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            CONFIRM_EXECUTION_TIME_ID,
            CONFIRM_EXECUTION_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            CACHE_PREEMPTIVE_EXECUTION_TIME_ID,
            CACHE_PREEMPTIVE_EXECUTION_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            CACHE_CONFIRM_APPLICATION_TIME_ID,
            CACHE_CONFIRM_APPLICATION_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            DS_PREEMPTIVE_EXECUTION_TIME_ID,
            DS_PREEMPTIVE_EXECUTION_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            DS_SPECULATION_TO_CONFIRM_LATENCY_ID,
            DS_SPECULATION_TO_CONFIRM_LATENCY.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            DS_BACKTRACK_COUNT_ID,
            DS_BACKTRACK_COUNT.to_string(),
            MetricKind::Counter,
            MetricLevel::Info,
        )
            .into(),
        (
            DS_OPS_PER_BATCH_ID,
            DS_OPS_PER_BATCH.to_string(),
            MetricKind::Count,
            MetricLevel::Debug,
        )
            .into(),
        (
            CACHE_SPECULATION_TO_CONFIRM_LATENCY_ID,
            CACHE_SPECULATION_TO_CONFIRM_LATENCY.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            CACHE_PENDING_QUEUE_SIZE_ID,
            CACHE_PENDING_QUEUE_SIZE.to_string(),
            MetricKind::Count,
            MetricLevel::Debug,
        )
            .into(),
        (
            CACHE_BACKTRACK_COUNT_ID,
            CACHE_BACKTRACK_COUNT.to_string(),
            MetricKind::Counter,
            MetricLevel::Info,
        )
            .into(),
        (
            CACHE_DELTA_SIZE_ID,
            CACHE_DELTA_SIZE.to_string(),
            MetricKind::Count,
            MetricLevel::Debug,
        )
            .into(),
        (
            CACHE_OPS_PER_BATCH_ID,
            CACHE_OPS_PER_BATCH.to_string(),
            MetricKind::Count,
            MetricLevel::Debug,
        )
            .into(),
        (
            CACHE_UNORDERED_EXECUTION_TIME_ID,
            CACHE_UNORDERED_EXECUTION_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            CACHE_REBUILD_TIME_ID,
            CACHE_REBUILD_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            CACHE_ENQUEUE_TO_EXECUTE_LATENCY_ID,
            CACHE_ENQUEUE_TO_EXECUTE_LATENCY.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            SCALABLE_PREEMPTIVE_EXECUTION_TIME_ID,
            SCALABLE_PREEMPTIVE_EXECUTION_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            SCALABLE_COLLISION_COUNT_ID,
            SCALABLE_COLLISION_COUNT.to_string(),
            MetricKind::Count,
            MetricLevel::Debug,
        )
            .into(),
        (
            SCALABLE_COLLISION_RATE_ID,
            SCALABLE_COLLISION_RATE.to_string(),
            MetricKind::Count,
            MetricLevel::Debug,
        )
            .into(),
        (
            SCALABLE_OPS_PER_BATCH_ID,
            SCALABLE_OPS_PER_BATCH.to_string(),
            MetricKind::Count,
            MetricLevel::Debug,
        )
            .into(),
        (
            REORDER_BUFFER_SIZE_ID,
            REORDER_BUFFER_SIZE.to_string(),
            MetricKind::Count,
            MetricLevel::Debug,
        )
            .into(),
        (
            REORDER_STAGED_COUNT_ID,
            REORDER_STAGED_COUNT.to_string(),
            MetricKind::Counter,
            MetricLevel::Info,
        )
            .into(),
        (
            SPECULATION_FALLBACK_COUNT_ID,
            SPECULATION_FALLBACK_COUNT.to_string(),
            MetricKind::Counter,
            MetricLevel::Info,
        )
            .into(),
        (
            SCALABLE_SPECULATION_TO_CONFIRM_LATENCY_ID,
            SCALABLE_SPECULATION_TO_CONFIRM_LATENCY.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            SCALABLE_CONFIRM_APPLICATION_TIME_ID,
            SCALABLE_CONFIRM_APPLICATION_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            CONFIRM_ENQUEUE_TO_APPLY_LATENCY_ID,
            CONFIRM_ENQUEUE_TO_APPLY_LATENCY.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            CONFIRM_TO_REPLY_TIME_ID,
            CONFIRM_TO_REPLY_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            CONFIRM_BLOCKED_ON_EXEC_TIME_ID,
            CONFIRM_BLOCKED_ON_EXEC_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        // Info, not Debug, despite being a Count: this is the metric that says whether
        // speculation engaged at all, so it has to survive every level a suite may set.
        (
            SPECULATION_HIT_RATE_ID,
            SPECULATION_HIT_RATE.to_string(),
            MetricKind::Count,
            MetricLevel::Info,
        )
            .into(),
        // Three-tuple, so level defaults to Info — matching how atlas-smr-execution
        // registers the same two names. Info is the highest level and a metric is kept
        // when its own level is at or above the binary's, so these survive every
        // `with_metric_level` a suite might configure.
        (
            OPERATIONS_EXECUTED_PER_SECOND_ID,
            OPERATIONS_EXECUTED_PER_SECOND.to_string(),
            MetricKind::Counter,
        )
            .into(),
        (
            UNORDERED_OPS_PER_SECOND_ID,
            UNORDERED_OPS_PER_SECOND.to_string(),
            MetricKind::Counter,
        )
            .into(),
    ]
}
