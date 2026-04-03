use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};

pub const CONFIRMED_WORKER_LATENCY: &str = "CONFIRMED_WORKER_LATENCY";
pub const CONFIRMED_WORKER_LATENCY_ID: usize = 800;

pub const CONFIRM_EXECUTION_TIME: &str = "CONFIRM_EXECUTION_TIME";
pub const CONFIRM_EXECUTION_TIME_ID: usize = 801;

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
    ]
}
