use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};

pub(crate) const RQ_SEND_TIME: &str = "REQUEST_SEND_TIME";
pub(crate) const RQ_SEND_TIME_ID: usize = 1000;

pub(crate) const MESSAGES_IN_CHANNEL: &str = "MESSAGES_IN_CHANNEL";
pub(crate) const MESSAGES_IN_CHANNEL_ID: usize = 1001;

pub(crate) const MESSAGE_DISPATCH_TIME: &str = "MESSAGE_DISPATCH_TIME";
pub(crate) const MESSAGE_DISPATCH_TIME_ID: usize = 1002;

pub(crate) const MESSAGE_WAKER_TIME: &str = "MESSAGE_WAKER_TIME";
pub(crate) const MESSAGE_WAKER_TIME_ID: usize = 1003;

pub fn metrics() -> Vec<MetricRegistry> {
    vec![
        (
            RQ_SEND_TIME_ID,
            RQ_SEND_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
        (
            MESSAGES_IN_CHANNEL_ID,
            MESSAGES_IN_CHANNEL.to_string(),
            MetricKind::Count,
            MetricLevel::Trace,
        )
            .into(),
        (
            MESSAGE_DISPATCH_TIME_ID,
            MESSAGE_DISPATCH_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
        (
            MESSAGE_WAKER_TIME_ID,
            MESSAGE_WAKER_TIME.to_string(),
            MetricKind::Duration,
            MetricLevel::Trace,
        )
            .into(),
    ]
}
