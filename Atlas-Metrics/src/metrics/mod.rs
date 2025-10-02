use std::cell::Cell;
use std::fmt::{Debug, Formatter};
use std::iter;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use getset::Getters;
use thread_local::ThreadLocal;
use tracing::error;

use crate::metrics::correlation_counter::CorrelationCounterTracker;
use crate::metrics::correlation_ids::{
    encapsulate_correlation_id, end_correlation_id, pass_correlation_id, register_correlation_id,
    CorrelationTracker,
};
use crate::metrics::correlation_time::CorrelationTimeTracker;
use crate::{MetricLevel, MetricRegistry, MetricRegistryInfo};

mod correlation_counter;
mod correlation_ids;
mod correlation_time;
pub mod metrics_thread;
pub(super) mod os_mon;

static METRICS: OnceLock<Metrics> = OnceLock::new();

/// Metrics for the general library.
pub struct Metrics {
    live_indexes: Vec<usize>,
    metrics: Vec<Option<Metric>>,
    current_level: MetricLevel,
}

/// A metric statistic to be used. This will be collected every X seconds and sent to InfluxDB
#[derive(Getters)]
#[get = "pub"]
pub struct Metric {
    name: String,
    value: SafeMetricData,
    additional_data: Mutex<AdditionalMetricData>,
    metric_level: MetricLevel,
    metric_type: MetricKind,
}

/// The data for a metric
#[allow(clippy::large_enum_variant)]
enum SafeMetricData {
    Sequential(Vec<Mutex<MetricData>>, ThreadLocal<Cell<usize>>),
    ThreadSafe(RwLock<MetricData>),
    Atomic(MetricData),
}

/// Data for a given metric
#[derive(Debug)]
pub(super) enum MetricData {
    /// This uses Welford's method for rolling standard dev calculations
    Duration {
        m: AtomicI64,
        s: AtomicI64,
        count: AtomicUsize,
        sum: AtomicI64,
    },
    /// A counter is a metric that is incremented by X every time it is called and in the end is combined
    Counter(AtomicU64),

    CounterCorrelation(CorrelationCounterTracker),
    /// A vector that stores the counts of things and then averages their values to feed to influx db
    /// This is useful for stuff like the batch size, which we don't want to add together.
    Count {
        m: AtomicI64,
        s: AtomicI64,
        count: AtomicUsize,
        sum: AtomicI64,
    },
    /// A counter that stores the maximum value
    CountMax(Vec<(u64, DateTime<Utc>)>),

    Correlation(CorrelationTracker),

    CorrelationDurationTracker(CorrelationTimeTracker),

    CorrelationAggrDurationTracker(CorrelationTimeTracker),
}

#[derive(Debug)]
/// Additional data that might be useful for a metric
/// For example, duration metrics need to know when they started for
/// when we are measuring durations that are not in the same scope (without having to
/// pass ugly arguments and such around)
pub enum AdditionalMetricData {
    Duration(Option<Instant>),
    // Counter does not need any additional data storage
    Counter,
    Count,
    Correlation,
}

/// The possible kinds of metrics
#[derive(Debug)]
pub enum MetricKind {
    Duration,
    /// A counter is a metric that is incremented by X every time it is called and in the end is combined
    Counter,
    CounterCorrelation,
    /// A count is to be used to store various independent counts and then average them together
    Count,
    /// Metrics for correlation ID tracking
    CountMax(usize),
    /// Metrics for correlation ID tracking
    Correlation,
    /// Duration with correlation, individually addressed
    CorrelationDurationTracker,
    /// Duration with correlation, aggregated
    CorrelationAggrDurationTracker,
}

impl Metrics {
    fn new(
        registered_metrics: Vec<MetricRegistry>,
        metric_level: MetricLevel,
        concurrency: usize,
    ) -> Self {
        let mut largest_ind = 0;

        // Calculate the necessary size for the vector
        for info in &registered_metrics {
            largest_ind = std::cmp::max(largest_ind, info.index);
        }

        let mut metrics = iter::repeat_with(|| None)
            .take(largest_ind + 1)
            .collect::<Vec<_>>();

        let mut live_indexes = Vec::with_capacity(registered_metrics.len());

        for metric in registered_metrics {
            let index = metric.index;

            metrics[index] = Some(Metric::new(
                metric.name,
                metric.kind,
                metric.level,
                metric.concurrency_override.unwrap_or(concurrency),
            ));

            live_indexes.push(index);
        }

        Self {
            live_indexes,
            metrics,
            current_level: metric_level,
        }
    }
}

impl MetricKind {
    fn gen_metric_type(&self, concurrency: usize) -> SafeMetricData {
        match self {
            MetricKind::CountMax(_) => {
                let buckets = iter::repeat_with(|| Mutex::new(self.gen_metric_type_internal()))
                    .take(concurrency)
                    .collect();

                SafeMetricData::Sequential(buckets, ThreadLocal::new())
            }
            MetricKind::Counter | MetricKind::Duration | MetricKind::Count => {
                let metric = self.gen_metric_type_internal();

                SafeMetricData::Atomic(metric)
            }
            _ => {
                let metric = self.gen_metric_type_internal();

                SafeMetricData::ThreadSafe(RwLock::new(metric))
            }
        }
    }

    fn gen_metric_type_internal(&self) -> MetricData {
        match self {
            MetricKind::Duration => MetricData::Duration {
                m: AtomicI64::new(0),
                s: AtomicI64::new(0),
                count: AtomicUsize::new(0),
                sum: AtomicI64::new(0),
            },
            MetricKind::Count => MetricData::Count {
                m: AtomicI64::new(0),
                s: AtomicI64::new(0),
                count: AtomicUsize::new(0),
                sum: AtomicI64::new(0),
            },
            MetricKind::Counter => MetricData::Counter(AtomicU64::new(0)),
            MetricKind::CountMax(_) => MetricData::CountMax(Vec::new()),
            MetricKind::Correlation => MetricData::Correlation(Default::default()),
            MetricKind::CorrelationDurationTracker => {
                MetricData::CorrelationDurationTracker(Default::default())
            }
            MetricKind::CorrelationAggrDurationTracker => {
                MetricData::CorrelationAggrDurationTracker(Default::default())
            }
            MetricKind::CounterCorrelation => MetricData::CounterCorrelation(Default::default()),
        }
    }

    fn gen_additional_data(&self) -> AdditionalMetricData {
        match self {
            MetricKind::Duration => AdditionalMetricData::Duration(None),
            MetricKind::Counter => AdditionalMetricData::Counter,
            MetricKind::Count => AdditionalMetricData::Count,
            MetricKind::CountMax(_) => AdditionalMetricData::Counter,
            MetricKind::Correlation => AdditionalMetricData::Correlation,
            MetricKind::CorrelationDurationTracker | MetricKind::CorrelationAggrDurationTracker => {
                AdditionalMetricData::Correlation
            }
            MetricKind::CounterCorrelation => AdditionalMetricData::Counter,
        }
    }
}

impl Metric {
    fn new(name: String, kind: MetricKind, level: MetricLevel, concurrency: usize) -> Self {
        Self {
            name,
            value: kind.gen_metric_type(concurrency),
            additional_data: Mutex::new(kind.gen_additional_data()),
            metric_level: level,
            metric_type: kind,
        }
    }

    fn take_values(&self) -> Vec<MetricData> {
        match &self.value {
            SafeMetricData::Sequential(data_buckets, _) => {
                let mut collected_values = Vec::with_capacity(data_buckets.len());

                data_buckets.iter().for_each(|data| {
                    let metric_type = {
                        let mut value = data.lock().unwrap();

                        let mt = self.metric_type.gen_metric_type_internal();

                        let prev_value = std::mem::replace(&mut *value, mt);

                        if let MetricData::CorrelationDurationTracker(tracker) = prev_value {
                            if let MetricData::CorrelationDurationTracker(curr_tracker) =
                                &mut *value
                            {
                                let CorrelationTimeTracker {
                                    time_track,
                                    accumulated,
                                } = tracker;

                                curr_tracker.time_track = time_track;

                                MetricData::CorrelationDurationTracker(CorrelationTimeTracker {
                                    time_track: Default::default(),
                                    accumulated,
                                })
                            } else {
                                unreachable!("How this metric data be replaced by something else?")
                            }
                        } else {
                            prev_value
                        }
                    };

                    collected_values.push(metric_type);
                });

                collected_values
            }
            SafeMetricData::ThreadSafe(reference) => {
                let mut previous_value_guard = reference.write().unwrap();

                let previous_value = std::mem::replace(
                    &mut *previous_value_guard,
                    self.metric_type.gen_metric_type_internal(),
                );

                let previous_value = match previous_value {
                    MetricData::CorrelationDurationTracker(tracker) => {
                        // Pass the time track to the new value as we want to keep the pending ones
                        let CorrelationTimeTracker {
                            time_track,
                            accumulated,
                        } = tracker;

                        let new_value =
                            MetricData::CorrelationDurationTracker(CorrelationTimeTracker {
                                time_track,
                                accumulated: Default::default(),
                            });

                        let _ = std::mem::replace(&mut *previous_value_guard, new_value);

                        MetricData::CorrelationDurationTracker(CorrelationTimeTracker {
                            time_track: Default::default(),
                            accumulated,
                        })
                    }
                    MetricData::CorrelationAggrDurationTracker(tracker) => {
                        // Pass the time track to the new value as we want to keep the pending ones
                        let CorrelationTimeTracker {
                            time_track,
                            accumulated,
                        } = tracker;

                        let new_value =
                            MetricData::CorrelationAggrDurationTracker(CorrelationTimeTracker {
                                time_track,
                                accumulated: Default::default(),
                            });

                        let _ = std::mem::replace(&mut *previous_value_guard, new_value);

                        MetricData::CorrelationAggrDurationTracker(CorrelationTimeTracker {
                            time_track: Default::default(),
                            accumulated,
                        })
                    }
                    _ => previous_value,
                };

                vec![previous_value]
            }
            SafeMetricData::Atomic(data) => {
                let data = match data {
                    MetricData::Counter(counter) => {
                        MetricData::Counter(AtomicU64::new(counter.swap(0, Ordering::Relaxed)))
                    }
                    MetricData::Duration { s, m, sum, count } => {
                        let s = AtomicI64::new(s.swap(0, Ordering::Relaxed));
                        let m = AtomicI64::new(m.swap(0, Ordering::Relaxed));
                        let sum = AtomicI64::new(sum.swap(0, Ordering::Relaxed));
                        let count = AtomicUsize::new(count.swap(0, Ordering::Relaxed));

                        MetricData::Duration { s, m, sum, count }
                    }
                    MetricData::Count { s, m, sum, count } => {
                        let s = AtomicI64::new(s.swap(0, Ordering::Relaxed));
                        let m = AtomicI64::new(m.swap(0, Ordering::Relaxed));
                        let sum = AtomicI64::new(sum.swap(0, Ordering::Relaxed));
                        let count = AtomicUsize::new(count.swap(0, Ordering::Relaxed));

                        MetricData::Count { s, m, sum, count }
                    }
                    _ => unreachable!(""),
                };

                vec![data]
            }
        }
    }
}

impl MetricData {
    fn merge(&mut self, other: Self) {
        match (self, other) {
            (MetricData::Counter(c), MetricData::Counter(c2)) => {
                c.fetch_add(c2.load(Ordering::Relaxed), Ordering::Relaxed);
            }
            (MetricData::CountMax(count), MetricData::CountMax(mut count2)) => {
                count.append(&mut count2);
            }
            _ => panic!("Can't merge metrics of different types"),
        }
    }
}

impl SafeMetricData {
    fn get_round_robin(&self) -> usize {
        match self {
            SafeMetricData::Sequential(_, round_robin) => {
                let cell = round_robin.get_or(|| Cell::new(0));

                let current_round_robin = cell.get();

                cell.set(current_round_robin + 1);

                current_round_robin
            }
            SafeMetricData::ThreadSafe(_) | SafeMetricData::Atomic(_) => 0,
        }
    }

    pub fn locked_metric_data(&'_ self) -> MutexGuard<'_, MetricData> {
        match self {
            SafeMetricData::Sequential(data, _) => {
                let round_robin = self.get_round_robin();

                data[round_robin % data.len()].lock().unwrap()
            }
            SafeMetricData::ThreadSafe(_) => {
                unreachable!("Cannot get a lock on a thread safe metric")
            }
            SafeMetricData::Atomic(_) => {
                unreachable!("Cannot get a lock on an atomic metric")
            }
        }
    }

    pub fn get_thread_safe_read(&'_ self) -> RwLockReadGuard<'_, MetricData> {
        match self {
            SafeMetricData::ThreadSafe(data) => data.read().unwrap(),
            SafeMetricData::Sequential(_, _) => {
                unreachable!("Cannot get a reference to a sequential metric")
            }
            SafeMetricData::Atomic(_) => {
                unreachable!("Cannot get a reference to an atomic metric")
            }
        }
    }

    pub fn get_metric_data(&self) -> &MetricData {
        match self {
            SafeMetricData::Atomic(data) => data,
            SafeMetricData::ThreadSafe(_) => {
                unreachable!("Cannot get a reference to a thread safe metric")
            }
            SafeMetricData::Sequential(_, _) => {
                unreachable!("Cannot get a reference to a sequential metric")
            }
        }
    }
}

/// Initialize the metrics module
pub(super) fn init(
    registered_metrics: Vec<MetricRegistryInfo>,
    concurrency: usize,
    level: MetricLevel,
) {
    let _ = METRICS.set(Metrics::new(registered_metrics, level, concurrency));
}

/// Enqueue a duration measurement
#[inline]
fn increase_welford_method(metric: &Metric, duration: u64) {
    let values = metric.value().get_metric_data();

    let duration = duration as i64;

    match values {
        MetricData::Duration { m, s, count, sum } | MetricData::Count { m, s, count, sum } => {
            // Welford's method for rolling standard deviation
            let k = count.fetch_add(1, Ordering::Relaxed) + 1;

            sum.fetch_add(duration, Ordering::Relaxed);

            let old_m = m
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |m| {
                    Some(m + (duration - m) / k as i64)
                })
                .expect("Failed to update sum");

            let new_m = old_m + (duration - old_m) / k as i64;

            s.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |s| {
                Some(
                    s.overflowing_add((duration - new_m).overflowing_mul(duration - old_m).0)
                        .0,
                )
            })
            .expect("Failed to update sum_sqrs");
        }
        _ => unreachable!("Metric is not a duration metric"),
    }
}

#[inline]
fn enqueue_counter_max_measurement(metric: &Metric, count: usize) {
    let mut values = metric.value().locked_metric_data();

    if let MetricData::CountMax(ref mut v) = *values {
        v.push((count as u64, Utc::now()));
    }
}

#[inline]
fn start_duration_measurement(metric: &Metric) {
    let mut metric_guard = metric.additional_data.lock().unwrap();

    match &mut *metric_guard {
        AdditionalMetricData::Duration(start) => {
            start.replace(Instant::now());
        }
        AdditionalMetricData::Counter => {}
        AdditionalMetricData::Count => {}
        AdditionalMetricData::Correlation => {}
    }
}

#[inline]
fn end_duration_measurement(metric: &Metric) {
    let start_instant = {
        let mut metric_guard = metric.additional_data.lock().unwrap();

        match &mut *metric_guard {
            AdditionalMetricData::Duration(start) => start.take(),
            AdditionalMetricData::Counter => None,
            AdditionalMetricData::Count => None,
            AdditionalMetricData::Correlation => None,
        }
    };

    if let Some(start_instant) = start_instant {
        increase_welford_method(metric, start_instant.elapsed().as_nanos() as u64);
    }
}

/// Enqueue a counter measurement
#[inline]
fn increment_counter_measurement(metric: &Metric, to_add: Option<u64>) {
    let values = metric.value().get_metric_data();

    if let MetricData::Counter(counter) = values {
        counter.fetch_add(to_add.unwrap_or(1), Ordering::Relaxed);
    } else {
        panic!("Metric is not a counter")
    }
}

/// Collect all measurements from a given metric
fn collect_measurements(metric: &Metric) -> Vec<MetricData> {
    metric.take_values()
}

/// Collect all measurements from all metrics
fn collect_all_measurements(level: &MetricLevel) -> Vec<(&Metric, MetricData)> {
    match METRICS.get() {
        Some(metrics) => {
            let mut collected_metrics = Vec::with_capacity(metrics.metrics.len());

            for index in &metrics.live_indexes {
                let metric = metrics.metrics[*index].as_ref().unwrap();

                if metric.metric_level < *level {
                    continue;
                }

                collected_metrics.push((metric, collect_measurements(metric)));
            }

            let mut final_metric_data = Vec::with_capacity(collected_metrics.len());

            for (name, metric_data) in collected_metrics {
                let joined_metrics = join_metrics(metric_data);

                final_metric_data.push((name, joined_metrics));
            }

            final_metric_data
        }
        None => panic!("Metrics were not initialized"),
    }
}

/// Join all metrics into a single metric so we can push them into InfluxDB
fn join_metrics(mut metrics: Vec<MetricData>) -> MetricData {
    if metrics.is_empty() {
        panic!("No metrics to join")
    }

    let mut first = metrics.swap_remove(0);

    while let Some(metric) = metrics.pop() {
        first.merge(metric)
    }

    first
}

fn metric_at_index<'a>(metrics: &&'a Metrics, index: usize) -> Option<&'a Metric> {
    metrics.metrics[index].as_ref()
}

#[inline]
pub fn metric_local_duration_start() -> Instant {
    Instant::now()
}

#[inline]
pub fn metric_local_duration_end(metric: usize, start: Instant) {
    metric_duration(metric, start.elapsed());
}

#[inline]
pub fn metric_duration_start(m_index: usize) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            start_duration_measurement(metric)
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_duration_end(m_index: usize) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            end_duration_measurement(metric)
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_duration(m_index: usize, duration: Duration) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            increase_welford_method(metric, duration.as_nanos() as u64);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_increment(m_index: usize, counter: Option<u64>) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            increment_counter_measurement(metric, counter);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_store_count(m_index: usize, amount: usize) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            increase_welford_method(metric, amount as u64);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_store_count_max(m_index: usize, amount: usize) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            enqueue_counter_max_measurement(metric, amount);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_initialize_correlation_id(
    m_index: usize,
    correlation_id: impl AsRef<str>,
    location: Arc<str>,
) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            let correlation_id = Arc::from(correlation_id.as_ref());

            register_correlation_id(metric, correlation_id, location);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_correlation_id_ended(
    m_index: usize,
    correlation_id: impl AsRef<str>,
    location: Arc<str>,
) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            let correlation_id = Arc::from(correlation_id.as_ref());

            end_correlation_id(metric, correlation_id, location);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_correlation_id_passed(
    m_index: usize,
    correlation_id: impl AsRef<str>,
    location: Arc<str>,
) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            let correlation_id = Arc::from(correlation_id.as_ref());

            pass_correlation_id(metric, correlation_id, location);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_correlation_id_encapsulated(
    m_index: usize,
    correlation_id: impl AsRef<str>,
    location: Arc<str>,
    (metric_id, encap_corr_id): (usize, impl AsRef<str>),
) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            let correlation_id = Arc::from(correlation_id.as_ref());

            encapsulate_correlation_id(
                metric,
                correlation_id,
                location,
                (metric_id, Arc::from(encap_corr_id.as_ref())),
            );
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_decapsulate_correlation_id(
    m_index: usize,
    correlation_id: impl AsRef<str>,
    location: Arc<str>,
) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            let correlation_id = Arc::from(correlation_id.as_ref());

            correlation_ids::decapsulate_correlation_id(metric, correlation_id, location);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_correlation_time_start(m_index: usize, correlation_id: impl AsRef<str>) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            let correlation_id = Arc::from(correlation_id.as_ref());

            correlation_time::start_correlation_time_tracker(metric, correlation_id);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_correlation_time_end(m_index: usize, correlation_id: impl AsRef<str>) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            let correlation_id = Arc::from(correlation_id.as_ref());

            correlation_time::end_correlation_time_tracker(metric, correlation_id);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

#[inline]
pub fn metric_correlation_counter_increment(
    m_index: usize,
    correlation_id: impl AsRef<str>,
    increment_count: Option<u64>,
) {
    let correlation_id = Arc::from(correlation_id.as_ref());

    metric_correlation_counter_increment_arc(m_index, correlation_id, increment_count)
}

#[inline]
pub fn metric_correlation_counter_increment_arc(
    m_index: usize,
    correlation_id: Arc<str>,
    increment_count: Option<u64>,
) {
    if let Some(ref metrics) = METRICS.get() {
        let metric = metric_at_index(metrics, m_index);

        if let Some(metric) = metric {
            if metric.metric_level < metrics.current_level {
                return;
            }

            correlation_counter::increment_counter(metric, correlation_id, increment_count);
        } else {
            error!(
                "Failed to get metric by index {}. It is probably not registered",
                m_index
            );
        }
    }
}

impl Debug for Metric {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metric")
            .field("name", &self.name)
            .field("metric_level", &self.metric_level)
            .finish()
    }
}
