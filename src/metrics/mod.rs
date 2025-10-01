use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};

/// Log transfer will take the 7XX metric ID range

pub const LOG_TRANSFER_LOG_CLONE_TIME: &str = "LT_STATE_CLONE_TIME";
pub const LOG_TRANSFER_LOG_CLONE_TIME_ID: usize = 700;

pub const LOG_TRANSFER_PROOFS_CLONE_TIME: &str = "LT_PROOFS_CLONE_TIME";
pub const LOG_TRANSFER_PROOFS_CLONE_TIME_ID: usize = 701;

pub fn metrics() -> Vec<MetricRegistry> {
    vec![
        (
            LOG_TRANSFER_LOG_CLONE_TIME_ID,
            LOG_TRANSFER_LOG_CLONE_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            LOG_TRANSFER_PROOFS_CLONE_TIME_ID,
            LOG_TRANSFER_PROOFS_CLONE_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
    ]
}
