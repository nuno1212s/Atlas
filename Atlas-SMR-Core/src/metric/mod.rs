use std::sync::Arc;

use lazy_static::lazy_static;

use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};

/// Request pre processing (10-19)
pub(crate) const RQ_PP_CLIENT_MSG: &str = "RQ_PRE_PROCESSING_CLIENT_MSGS";
pub(crate) const RQ_PP_CLIENT_MSG_ID: usize = 10;

pub(crate) const RQ_PP_CLIENT_COUNT: &str = "RQ_PRE_PROCESSING_CLIENT_COUNT";
pub(crate) const RQ_PP_CLIENT_COUNT_ID: usize = 11;

pub(crate) const RQ_PP_FWD_RQS: &str = "RQ_PRE_PROCESSING_FWD_RQS";
pub(crate) const RQ_PP_FWD_RQS_ID: usize = 12;

pub(crate) const RQ_PP_DECIDED_RQS: &str = "RQ_PRE_PROCESSING_DECIDED_RQS";
pub(crate) const RQ_PP_DECIDED_RQS_ID: usize = 13;

pub(crate) const RQ_PP_TIMEOUT_RQS: &str = "RQ_PRE_PROCESSING_TIMEOUT_RQS";
pub(crate) const RQ_PP_TIMEOUT_RQS_ID: usize = 14;

pub(crate) const RQ_PP_COLLECT_PENDING: &str = "RQ_PRE_PROCESSING_COLLECT_PENDING";
pub(crate) const RQ_PP_COLLECT_PENDING_ID: usize = 15;

pub(crate) const RQ_PP_CLONE_RQS: &str = "RQ_PRE_PROCESSING_CLONE_RQS";
pub(crate) const RQ_PP_CLONE_RQS_ID: usize = 16;

pub(crate) const RQ_PP_WORKER_ORDER_PROCESS: &str = "RQ_PRE_PROCESSING_WORKER_ORDERED_PROCESS";
pub(crate) const RQ_PP_WORKER_ORDER_PROCESS_ID: usize = 17;

pub(crate) const RQ_PP_WORKER_ORDER_PROCESS_COUNT: &str =
    "RQ_PRE_PROCESSING_WORKER_ORDERED_PROCESS_TIME";
pub(crate) const RQ_PP_WORKER_ORDER_PROCESS_COUNT_ID: usize = 18;

pub(crate) const RQ_PP_WORKER_DECIDED_PROCESS_TIME: &str =
    "RQ_PRE_PROCESSING_WORKER_DECIDED_PROCESS_TIME";
pub(crate) const RQ_PP_WORKER_DECIDED_PROCESS_TIME_ID: usize = 19;

pub(crate) const RQ_PP_ORCHESTRATOR_WORKER_PASSING_TIME: &str =
    "RQ_PRE_PROCESSING_ORCHESTRATOR_WORKER_PASSING_TIME";
pub(crate) const RQ_PP_ORCHESTRATOR_WORKER_PASSING_TIME_ID: usize = 20;

pub(crate) const RQ_PP_WORKER_STOPPED_TIME: &str = "RQ_PRE_PROCESSING_WORKER_STOPPED_TIME";
pub(crate) const RQ_PP_WORKER_STOPPED_TIME_ID: usize = 22;

pub(crate) const RQ_PP_WORKER_BATCH_SIZE: &str = "RQ_PRE_PROCESSING_BATCH_SIZE";
pub(crate) const RQ_PP_WORKER_BATCH_SIZE_ID: usize = 23;

pub(crate) const RQ_PP_WORKER_DISCARDED_RQS: &str = "RQ_PRE_PROCESSING_DISCARDED_REQUESTS";
pub(crate) const RQ_PP_WORKER_DISCARDED_RQS_ID: usize = 24;

pub(crate) const RQ_PP_ORCHESTRATOR_MESSAGES_PROCESSED: &str =
    "RQ_PRE_PROCESS_ORCHESTRATOR_MESSAGES_PROCESSED";
pub(crate) const RQ_PP_ORCHESTRATOR_MESSAGES_PROCESSED_ID: usize = 25;

pub fn metrics() -> Vec<MetricRegistry> {
    vec![
        (
            RQ_PP_CLIENT_MSG_ID,
            RQ_PP_CLIENT_MSG.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_CLIENT_COUNT_ID,
            RQ_PP_CLIENT_COUNT.to_string(),
            MetricKind::Counter,
        )
            .into(),
        (
            RQ_PP_FWD_RQS_ID,
            RQ_PP_FWD_RQS.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_DECIDED_RQS_ID,
            RQ_PP_DECIDED_RQS.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_TIMEOUT_RQS_ID,
            RQ_PP_TIMEOUT_RQS.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_COLLECT_PENDING_ID,
            RQ_PP_COLLECT_PENDING.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_CLONE_RQS_ID,
            RQ_PP_CLONE_RQS.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_WORKER_ORDER_PROCESS_ID,
            RQ_PP_WORKER_ORDER_PROCESS.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_WORKER_ORDER_PROCESS_COUNT_ID,
            RQ_PP_WORKER_ORDER_PROCESS_COUNT.to_string(),
            MetricKind::Counter,
        )
            .into(),
        (
            RQ_PP_WORKER_DECIDED_PROCESS_TIME_ID,
            RQ_PP_WORKER_DECIDED_PROCESS_TIME.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_ORCHESTRATOR_WORKER_PASSING_TIME_ID,
            RQ_PP_ORCHESTRATOR_WORKER_PASSING_TIME.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            RQ_PP_WORKER_STOPPED_TIME_ID,
            RQ_PP_WORKER_STOPPED_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            RQ_PP_WORKER_BATCH_SIZE_ID,
            RQ_PP_WORKER_BATCH_SIZE.to_string(),
            MetricKind::Count,
            MetricLevel::Info,
        )
            .into(),
        (
            RQ_PP_WORKER_DISCARDED_RQS_ID,
            RQ_PP_WORKER_DISCARDED_RQS.to_string(),
            MetricKind::Counter,
            MetricLevel::Debug,
        )
            .into(),
        (
            RQ_PP_ORCHESTRATOR_MESSAGES_PROCESSED_ID,
            RQ_PP_ORCHESTRATOR_MESSAGES_PROCESSED.to_string(),
            MetricKind::Counter,
            MetricLevel::Debug,
        )
            .into(),
    ]
}

lazy_static! {
    pub static ref CLIENT_RQ_ENTER_RQ_PRE_PROCESSOR: Arc<str> = Arc::from("ENTER_RQ_PRE_PROCESSOR");
}
