use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};

/// View Transfer protocol will take the 9XX

pub const VIEW_TRANSFER_PROCESS_MESSAGE_TIME: &str = "VT_MSG_PROCESS_TIME";
pub const VIEW_TRANSFER_PROCESS_MESSAGE_TIME_ID: usize = 900;

pub fn metrics() -> Vec<MetricRegistry> {
    vec![(
        VIEW_TRANSFER_PROCESS_MESSAGE_TIME_ID,
        VIEW_TRANSFER_PROCESS_MESSAGE_TIME.to_string(),
        MetricKind::Duration,
        MetricLevel::Info,
    )
        .into()]
}
