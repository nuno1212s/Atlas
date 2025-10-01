use crate::metrics::{Metric, MetricData};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

#[allow(dead_code)]
pub(crate) enum CorrelationEvent {
    Initialized,
    Passed,
    Encapsulated(usize, Arc<str>),
    Decapsulated,
    Ended,
}

pub(crate) struct CorrelationEventOccurrence {
    pub(super) date: DateTime<Utc>,
    pub(super) event: CorrelationEvent,
    pub(super) location: Arc<str>,
}

#[derive(Default)]
pub(crate) struct CorrelationTracker {
    pub(crate) map: DashMap<Arc<str>, Vec<CorrelationEventOccurrence>>,
}

impl Debug for CorrelationTracker {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "CorrelationTracker")
    }
}

impl Debug for CorrelationEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CorrelationEvent::Initialized => write!(f, "Initialized"),
            CorrelationEvent::Passed => write!(f, "Passed"),
            CorrelationEvent::Encapsulated(_, _) => write!(f, "Encapsulated"),
            CorrelationEvent::Decapsulated => write!(f, "Decapsulated"),
            CorrelationEvent::Ended => write!(f, "Ended"),
        }
    }
}

fn with_correlation_tracker(
    metric: &Metric,
    id: Arc<str>,
    f: impl FnOnce(&mut Vec<CorrelationEventOccurrence>),
) {
    let lock_guard = metric.value().get_thread_safe_read();

    if let MetricData::Correlation(tracker) = &*lock_guard {
        let mut entry = tracker.map.entry(id).or_insert_with(Vec::new);

        let correlation_id_events = entry.value_mut();

        f(correlation_id_events)
    } else {
        unreachable!("Metric {:?} is not a Correlation metric", metric)
    }
}

pub(crate) fn register_correlation_id(metric: &Metric, id: Arc<str>, location: Arc<str>) {
    with_correlation_tracker(metric, id, |correlation_id_events| {
        correlation_id_events.push(CorrelationEventOccurrence {
            date: Utc::now(),
            event: CorrelationEvent::Initialized,
            location,
        });
    });
}

pub(crate) fn pass_correlation_id(metric: &Metric, id: Arc<str>, location: Arc<str>) {
    with_correlation_tracker(metric, id, |correlation_id_events| {
        correlation_id_events.push(CorrelationEventOccurrence {
            date: Utc::now(),
            event: CorrelationEvent::Passed,
            location,
        });
    });
}

pub(crate) fn encapsulate_correlation_id(
    metric: &Metric,
    id: Arc<str>,
    location: Arc<str>,
    (encapsulating_metric_id, correlation_id): (usize, Arc<str>),
) {
    with_correlation_tracker(metric, id, |correlation_id_events| {
        correlation_id_events.push(CorrelationEventOccurrence {
            date: Utc::now(),
            event: CorrelationEvent::Encapsulated(encapsulating_metric_id, correlation_id),
            location,
        });
    });
}

pub(crate) fn decapsulate_correlation_id(metric: &Metric, id: Arc<str>, location: Arc<str>) {
    with_correlation_tracker(metric, id, |correlation_id_events| {
        correlation_id_events.push(CorrelationEventOccurrence {
            date: Utc::now(),
            event: CorrelationEvent::Decapsulated,
            location,
        });
    });
}

pub(crate) fn end_correlation_id(metric: &Metric, id: Arc<str>, location: Arc<str>) {
    with_correlation_tracker(metric, id, |correlation_id_events| {
        correlation_id_events.push(CorrelationEventOccurrence {
            date: Utc::now(),
            event: CorrelationEvent::Ended,
            location,
        });
    });
}
