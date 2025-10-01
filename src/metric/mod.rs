use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};

pub(crate) const COMM_SERIALIZE_SIGN_TIME: &str = "COMM_SERIALIZE_AND_SIGN_TIME";
pub(crate) const COMM_SERIALIZE_SIGN_TIME_ID: usize = 402;

pub(crate) const COMM_DESERIALIZE_VERIFY_TIME: &str = "COMM_DESERIALIZE_AND_VERIFY_TIME";
pub(crate) const COMM_DESERIALIZE_VERIFY_TIME_ID: usize = 403;

pub(crate) const CLIENT_POOL_COLLECT_TIME: &str = "CLIENT_POOL_COLLECT_TIME";
pub(crate) const CLIENT_POOL_COLLECT_TIME_ID: usize = 404;

pub(crate) const CLIENT_POOL_BATCH_PASSING_TIME: &str = "CLIENT_POOL_BATCH_PASSING_TIME";
pub(crate) const CLIENT_POOL_BATCH_PASSING_TIME_ID: usize = 405;

pub(crate) const THREADPOOL_PASS_TIME: &str = "THREADPOOL_PASS_TIME";
pub(crate) const THREADPOOL_PASS_TIME_ID: usize = 408;

pub(crate) const CLIENT_POOL_SLEEP_TIME: &str = "CLIENT_POOL_SLEEP_TIME";
pub(crate) const CLIENT_POOL_SLEEP_TIME_ID: usize = 409;

pub(crate) const RQ_CLIENT_POOL_TIME_SPENT: &str = "TIME_SPENT_CLIENT_POOL";
pub(crate) const RQ_CLIENT_POOL_TIME_SPENT_ID: usize = 410;

pub(crate) const OUTGOING_MESSAGE_SIZE: &str = "OUTGOING_MESSAGE_SIZE";
pub(crate) const OUTGOING_MESSAGE_SIZE_ID: usize = 411;

pub(crate) const INCOMING_MESSAGE_SIZE: &str = "INCOMING_MESSAGE_SIZE";
pub(crate) const INCOMING_MESSAGE_SIZE_ID: usize = 412;

pub(crate) const MESSAGE_DELIVER_TIME: &str = "MESSAGE_DELIVER_TIME";
pub(crate) const MESSAGE_DELIVER_TIME_ID: usize = 413;
pub fn metrics() -> Vec<MetricRegistry> {
    vec![
        (
            COMM_SERIALIZE_SIGN_TIME_ID,
            COMM_SERIALIZE_SIGN_TIME.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            COMM_DESERIALIZE_VERIFY_TIME_ID,
            COMM_DESERIALIZE_VERIFY_TIME.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            CLIENT_POOL_COLLECT_TIME_ID,
            CLIENT_POOL_COLLECT_TIME.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            CLIENT_POOL_BATCH_PASSING_TIME_ID,
            CLIENT_POOL_BATCH_PASSING_TIME.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            THREADPOOL_PASS_TIME_ID,
            THREADPOOL_PASS_TIME.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            CLIENT_POOL_SLEEP_TIME_ID,
            CLIENT_POOL_SLEEP_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
        (
            RQ_CLIENT_POOL_TIME_SPENT_ID,
            RQ_CLIENT_POOL_TIME_SPENT.to_string(),
            MetricKind::Duration,
        )
            .into(),
        (
            OUTGOING_MESSAGE_SIZE_ID,
            OUTGOING_MESSAGE_SIZE.to_string(),
            MetricKind::CountMax(10),
            MetricLevel::Trace,
        )
            .into(),
        (
            INCOMING_MESSAGE_SIZE_ID,
            INCOMING_MESSAGE_SIZE.to_string(),
            MetricKind::CountMax(10),
            MetricLevel::Trace,
        )
            .into(),
        (
            MESSAGE_DELIVER_TIME_ID,
            MESSAGE_DELIVER_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
    ]
}
