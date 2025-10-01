use crate::metrics::{Metric, MetricData};
use dashmap::DashMap;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct CorrelationTimeTracker {
    pub(super) time_track: DashMap<Arc<str>, Instant>,
    pub(super) accumulated: DashMap<Arc<str>, Duration>,
}

pub(crate) fn start_correlation_time_tracker(metric: &Metric, id: Arc<str>) {
    let lock_guard = metric.value().get_thread_safe_read();
    match &*lock_guard {
        MetricData::CorrelationDurationTracker(tracker)
        | MetricData::CorrelationAggrDurationTracker(tracker) => {
            tracker.time_track.insert(id, Instant::now());
        }
        _ => unreachable!("Metric {:?} is not a CorrelationTime metric", metric),
    }
}

pub(crate) fn end_correlation_time_tracker(metric: &Metric, id: Arc<str>) {
    let lock_guard = metric.value().get_thread_safe_read();
    match &*lock_guard {
        MetricData::CorrelationDurationTracker(tracker)
        | MetricData::CorrelationAggrDurationTracker(tracker) => {
            if let Some((correlation, init_time)) = tracker.time_track.remove(&id) {
                tracker.accumulated.insert(correlation, init_time.elapsed());
            }
        }
        _ => unreachable!("Metric {:?} is not a CorrelationTime metric", metric),
    }
}

impl Debug for CorrelationTimeTracker {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "CorrelationTimeTracker")
    }
}
