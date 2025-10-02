use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};
use lazy_static::lazy_static;
use std::sync::Arc;

// Replica will get the 5XX metrics codes

/// Time taken for the replica to poll the ordering protocol
pub(crate) const ORDERING_PROTOCOL_POLL_TIME: &str = "ORDERING_PROTOCOL_POLL_TIME";
pub(crate) const ORDERING_PROTOCOL_POLL_TIME_ID: usize = 500;

/// Time taken by the ordering protocol to process a message
pub(crate) const ORDERING_PROTOCOL_PROCESS_TIME: &str = "ORDERING_PROTOCOL_PROCESS_TIME";
pub(crate) const ORDERING_PROTOCOL_PROCESS_TIME_ID: usize = 501;

/// Time taken to process a given state transfer received work
pub(crate) const STATE_TRANSFER_PROCESS_TIME: &str = "STATE_TRANSFER_PROCESS_TIME";
pub(crate) const STATE_TRANSFER_PROCESS_TIME_ID: usize = 502;

/// Time taken to process a timeout
pub(crate) const TIMEOUT_PROCESS_TIME: &str = "TIMEOUT_PROCESS_TIME";
pub(crate) const TIMEOUT_PROCESS_TIME_ID: usize = 503;

pub(crate) const TIMEOUT_RECEIVED_COUNT: &str = "TIMEOUT_RECEIVED_COUNT";
pub(crate) const TIMEOUT_RECEIVED_COUNT_ID: usize = 504;

/// Time taken to digest app state received from the application
/// Not used in divisible state
pub(crate) const APP_STATE_DIGEST_TIME: &str = "APP_STATE_DIGEST_TIME";
pub(crate) const APP_STATE_DIGEST_TIME_ID: usize = 505;

/// Time taken to run the entirety of the regular replica protocols
/// (Unrelated to monolithic or divisible state)
pub(crate) const RUN_LATENCY_TIME: &str = "RUN_LATENCY_TIME";
pub(crate) const RUN_LATENCY_TIME_ID: usize = 509;

pub(crate) const REPLICA_RQ_QUEUE_SIZE: &str = "REPLICA_RQ_QUEUE_SIZE";
pub(crate) const REPLICA_RQ_QUEUE_SIZE_ID: usize = 510;

/// Time taken to process other replica protocols and other protocol messages (other than ordering)
pub(crate) const REPLICA_INTERNAL_PROCESS_TIME: &str = "REPLICA_INTERNAL_PROCESS_TIME";
pub(crate) const REPLICA_INTERNAL_PROCESS_TIME_ID: usize = 511;

/// Time taken for the replica to remove messages from the network
pub(crate) const REPLICA_TAKE_FROM_NETWORK: &str = "REPLICA_TAKE_FROM_NETWORK";
pub(crate) const REPLICA_TAKE_FROM_NETWORK_ID: usize = 512;

/// How many ordered operations are we processing (and pushing) to the executor
pub(crate) const REPLICA_ORDERED_RQS_PROCESSED: &str = "REPLICA_ORDERED_RQS_PROCESSED";
pub(crate) const REPLICA_ORDERED_RQS_PROCESSED_ID: usize = 513;

/// Time taken for the decision log thread to process some work
pub(crate) const DEC_LOG_PROCESS_TIME: &str = "DECISION_LOG_PROCESS_TIME";
pub(crate) const DEC_LOG_PROCESS_TIME_ID: usize = 514;

/// Time taken for the ordering protocol thread to deliver work to the decision log thread
pub(crate) const DEC_LOG_WORK_MSG_TIME: &str = "DECISION_LOG_WORK_DELIVER_TIME";
pub(crate) const DEC_LOG_WORK_MSG_TIME_ID: usize = 505;

pub(crate) const DEC_LOG_WORK_QUEUE_SIZE: &str = "DECISION_LOG_WORK_QUEUE_SIZE";
pub(crate) const DEC_LOG_WORK_QUEUE_SIZE_ID: usize = 506;

/// Time taken for the replica to process the response given by the ordering protocol to a processed
/// message
pub(crate) const REPLICA_PROTOCOL_RESP_PROCESS_TIME: &str = "REPLICA_PROTOCOL_RESP_PROCESS_TIME";
pub(crate) const REPLICA_PROTOCOL_RESP_PROCESS_TIME_ID: usize = 515;

/// How many unordered operations are we pushing to the executor
pub(crate) const UNORDERED_OPS_PER_SECOND: &str = "UNORDERED_OPS_PUSHED";
pub(crate) const UNORDERED_OPS_PER_SECOND_ID: usize = 516;

pub(crate) const OP_MESSAGES_PROCESSED: &str = "OP_MESSAGES_PROCESSED";
pub(crate) const OP_MESSAGES_PROCESSED_ID: usize = 517;

pub(crate) const DECISION_LOG_PROCESSED: &str = "DL_MESSAGES_PROCESSED";
pub(crate) const DECISION_LOG_PROCESSED_ID: usize = 518;

pub fn metrics() -> Vec<MetricRegistry> {
    vec![
        (
            ORDERING_PROTOCOL_POLL_TIME_ID,
            ORDERING_PROTOCOL_POLL_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
        (
            ORDERING_PROTOCOL_PROCESS_TIME_ID,
            ORDERING_PROTOCOL_PROCESS_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            STATE_TRANSFER_PROCESS_TIME_ID,
            STATE_TRANSFER_PROCESS_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            TIMEOUT_PROCESS_TIME_ID,
            TIMEOUT_PROCESS_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            TIMEOUT_RECEIVED_COUNT_ID,
            TIMEOUT_RECEIVED_COUNT.to_string(),
            MetricKind::Counter,
            MetricLevel::Debug,
        )
            .into(),
        (
            APP_STATE_DIGEST_TIME_ID,
            APP_STATE_DIGEST_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            RUN_LATENCY_TIME_ID,
            RUN_LATENCY_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            REPLICA_RQ_QUEUE_SIZE_ID,
            REPLICA_RQ_QUEUE_SIZE.to_string(),
            MetricKind::Count,
            MetricLevel::Trace,
        )
            .into(),
        (
            REPLICA_INTERNAL_PROCESS_TIME_ID,
            REPLICA_INTERNAL_PROCESS_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
        (
            REPLICA_TAKE_FROM_NETWORK_ID,
            REPLICA_TAKE_FROM_NETWORK.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
        (
            REPLICA_ORDERED_RQS_PROCESSED_ID,
            REPLICA_ORDERED_RQS_PROCESSED.to_string(),
            MetricKind::Counter,
            MetricLevel::Trace,
        )
            .into(),
        (
            DEC_LOG_PROCESS_TIME_ID,
            DEC_LOG_PROCESS_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            DEC_LOG_WORK_MSG_TIME_ID,
            DEC_LOG_WORK_MSG_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
        (
            DEC_LOG_WORK_QUEUE_SIZE_ID,
            DEC_LOG_WORK_QUEUE_SIZE.to_string(),
            MetricKind::Count,
            MetricLevel::Trace,
        )
            .into(),
        (
            REPLICA_PROTOCOL_RESP_PROCESS_TIME_ID,
            REPLICA_PROTOCOL_RESP_PROCESS_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Debug,
        )
            .into(),
        (
            STATE_TRANSFER_PROCESS_TIME_ID,
            STATE_TRANSFER_PROCESS_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Info,
        )
            .into(),
        (
            UNORDERED_OPS_PER_SECOND_ID,
            UNORDERED_OPS_PER_SECOND.to_string(),
            MetricKind::Counter,
        )
            .into(),
        (
            OP_MESSAGES_PROCESSED_ID,
            OP_MESSAGES_PROCESSED.to_string(),
            MetricKind::Counter,
            MetricLevel::Info,
        )
            .into(),
        (
            DECISION_LOG_PROCESSED_ID,
            DECISION_LOG_PROCESSED.to_string(),
            MetricKind::Counter,
            MetricLevel::Info,
        )
            .into(),
    ]
}

lazy_static! {
    pub static ref PASSED_TO_DECISION_LOG: Arc<str> = Arc::from("PASS_DECISION_LOG");
    pub static ref RECEIVED_FROM_DECISION_LOG: Arc<str> = Arc::from("END_DECISION_LOG");
}
