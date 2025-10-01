use atlas_metrics::metrics::MetricKind;
use atlas_metrics::{MetricLevel, MetricRegistry};

pub(crate) const DECISION_LOG_CHECKPOINT_TIME: &str = "DEC_LOG_CHECKPOINT_TIME";
pub(crate) const DECISION_LOG_CHECKPOINT_TIME_ID: usize = 900;

#[allow(dead_code)]
pub fn metrics() -> Vec<MetricRegistry> {
    vec![(
        DECISION_LOG_CHECKPOINT_TIME_ID,
        DECISION_LOG_CHECKPOINT_TIME.to_string(),
        MetricKind::Duration,
        MetricLevel::Info,
    )
        .into()]
}
