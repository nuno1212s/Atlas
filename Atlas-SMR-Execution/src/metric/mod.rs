use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};
use lazy_static::lazy_static;
use std::sync::Arc;

/// SMR Execution will get 8XX
pub const EXECUTION_LATENCY_TIME: &str = "EXECUTION_LATENCY";
pub const EXECUTION_LATENCY_TIME_ID: usize = 800;

/// Time taken to execute a batch of operations
pub const EXECUTION_TIME_TAKEN: &str = "EXECUTION_TIME_TAKEN";
pub const EXECUTION_TIME_TAKEN_ID: usize = 801;

pub const REPLIES_SENT_TIME: &str = "REPLY_SENT_TIME";
pub const REPLIES_SENT_TIME_ID: usize = 802;

pub const REPLIES_PASSING_TIME: &str = "REPLIES_PASSING_TIME";
pub const REPLIES_PASSING_TIME_ID: usize = 803;

pub const OPERATIONS_EXECUTED_PER_SECOND: &str = "OPERATIONS_EXECUTED_PER_SECOND";
pub const OPERATIONS_EXECUTED_PER_SECOND_ID: usize = 804;

pub const UNORDERED_OPS_PER_SECOND: &str = "UNORDERED_OPERATIONS_EXECUTED_PER_SECOND";
pub const UNORDERED_OPS_PER_SECOND_ID: usize = 805;

pub const UNORDERED_EXECUTION_TIME_TAKEN: &str = "UNORDERED_EXECUTION_TIME_TAKEN";
pub const UNORDERED_EXECUTION_TIME_TAKEN_ID: usize = 806;

pub fn metrics() -> Vec<MetricRegistry> {
    vec![
        (
            EXECUTION_LATENCY_TIME_ID,
            EXECUTION_LATENCY_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            EXECUTION_TIME_TAKEN_ID,
            EXECUTION_TIME_TAKEN.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            UNORDERED_EXECUTION_TIME_TAKEN_ID,
            UNORDERED_EXECUTION_TIME_TAKEN.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            REPLIES_SENT_TIME_ID,
            REPLIES_SENT_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            REPLIES_PASSING_TIME_ID,
            REPLIES_PASSING_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
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

lazy_static! {
    pub static ref REPLYING_TO_REQUEST: Arc<str> = Arc::from("REPLY_TO_REQUEST");
}
