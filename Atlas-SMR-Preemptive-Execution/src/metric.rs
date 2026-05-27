use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};

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
// Scalable CRUD executor metrics (817-818)
// ---------------------------------------------------------------------------

/// Total wall-clock time for one parallel+collision+reexec cycle in the scalable CRUD executor.
pub const SCALABLE_PREEMPTIVE_EXECUTION_TIME: &str = "SCALABLE_PREEMPTIVE_EXECUTION_TIME";
pub const SCALABLE_PREEMPTIVE_EXECUTION_TIME_ID: usize = 817;

/// Number of colliding operations per batch in the scalable CRUD executor.
pub const SCALABLE_COLLISION_COUNT: &str = "SCALABLE_COLLISION_COUNT";
pub const SCALABLE_COLLISION_COUNT_ID: usize = 818;

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
    ]
}
