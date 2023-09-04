use atlas_metrics::{MetricLevel, MetricRegistry};
use atlas_metrics::metrics::MetricKind;

pub const EXECUTION_LATENCY_TIME: &str = "EXECUTION_LATENCY";
pub const EXECUTION_LATENCY_TIME_ID: usize = 505;

pub const EXECUTION_TIME_TAKEN: &str = "EXECUTION_TIME_TAKEN";
pub const EXECUTION_TIME_TAKEN_ID: usize = 506;

pub const REPLIES_SENT_TIME: &str = "REPLY_SENT_TIME";
pub const REPLIES_SENT_TIME_ID: usize = 507;

pub const REPLIES_PASSING_TIME: &str = "REPLIES_PASSING_TIME";
pub const REPLIES_PASSING_TIME_ID: usize = 508;

pub fn metrics() -> Vec<MetricRegistry> {

    vec![
        (EXECUTION_LATENCY_TIME_ID, EXECUTION_LATENCY_TIME.to_string(), MetricKind::Duration, MetricLevel::Debug).into(),
        (EXECUTION_TIME_TAKEN_ID, EXECUTION_TIME_TAKEN.to_string(), MetricKind::Duration, MetricLevel::Debug).into(),
        (REPLIES_SENT_TIME_ID, REPLIES_SENT_TIME.to_string(), MetricKind::Duration, MetricLevel::Debug).into(),
        (REPLIES_PASSING_TIME_ID, REPLIES_PASSING_TIME.to_string(), MetricKind::Duration, MetricLevel::Debug).into(),
    ]

}