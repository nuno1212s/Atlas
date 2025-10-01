use std::iter;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{DateTime, Utc};
use influxdb::{InfluxDbWriteable, WriteQuery};
use tracing::{debug, error, info};

use crate::metrics::correlation_ids::CorrelationEventOccurrence;
use crate::metrics::{collect_all_measurements, MetricData, MetricKind};
use crate::{InfluxDBArgs, MetricLevel};
use atlas_common::async_runtime as rt;
use atlas_common::maybe_vec::MaybeVec;

#[derive(InfluxDbWriteable)]
pub struct MetricCounterReading {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    value: i64,
}

#[derive(InfluxDbWriteable)]
pub struct MetricDurationReading {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    value: f64,
    std_dev: f64,
}

#[derive(InfluxDbWriteable)]
pub struct MetricCountMaxReading {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    value: i64,
}

#[derive(InfluxDbWriteable)]
pub struct MetricCorrelationReading {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    correlation_id: String,
    location: String,
    #[influxdb(tag)]
    event: String,
}

#[derive(InfluxDbWriteable)]
pub struct MetricCorrelationTimeReading {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    correlation_id: String,
    value: i64,
}

#[derive(InfluxDbWriteable)]
pub struct MetricCorrelationAggrTimeReading {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    value: i64,
}

#[derive(InfluxDbWriteable)]
pub struct MetricCorrelationCounterReading {
    time: DateTime<Utc>,
    #[influxdb(tag)]
    host: String,
    #[influxdb(tag)]
    extra: String,
    correlation_id: String,
    value: i64,
}

pub fn launch_metrics(influx_args: InfluxDBArgs, metric_level: MetricLevel) {
    std::thread::spawn(move || {
        metric_thread_loop(influx_args, metric_level);
    });
}

/// The metrics thread. Collects all values from the metrics and sends them to influx db
pub fn metric_thread_loop(influx_args: InfluxDBArgs, metric_level: MetricLevel) {
    let InfluxDBArgs {
        ip,
        db_name,
        user,
        password,
        node_id,
        extra,
    } = influx_args;

    info!("Initializing metrics thread with test name: {:?}", extra);

    let mut client = influxdb::Client::new(ip.to_string(), db_name);

    client = client.with_auth(user, password);

    let host_name = format!("{node_id:?}");

    let extra = extra.unwrap_or(String::from("None"));

    loop {
        let measurements = collect_all_measurements(&metric_level);

        let time = Utc::now();

        let mut readings = Vec::with_capacity(measurements.len());

        for (metric, results) in measurements {
            let metric_name = metric.name();
            let query = match results {
                MetricData::Duration { s, count, sum, .. }
                | MetricData::Count { s, count, sum, .. } => {
                    let s = s.load(Ordering::Relaxed) as f64;
                    let count = count.load(Ordering::Relaxed) as f64;
                    let sum = sum.load(Ordering::Relaxed) as f64;

                    if count < 1f64 {
                        continue;
                    }

                    let avg = sum / count;

                    let std_dev = if count >= 2f64 {
                        s / (count - 1f64)
                    } else {
                        0f64
                    };

                    MaybeVec::from_one(
                        MetricDurationReading {
                            time,
                            host: host_name.clone(),
                            extra: extra.clone(),
                            value: avg,
                            std_dev,
                        }
                        .into_query(metric_name),
                    )
                }
                MetricData::Counter(count) => {
                    MaybeVec::from_one(
                        MetricCounterReading {
                            time,
                            host: host_name.clone(),
                            extra: extra.clone(),
                            // Could lose some information, but the driver did NOT like u64
                            value: count.load(Ordering::Relaxed) as i64,
                        }
                        .into_query(metric_name),
                    )
                }
                MetricData::Correlation(corr_data) => corr_data
                    .map
                    .iter_mut()
                    .flat_map(|mut item| {
                        let corr_id = item.key().clone();

                        let guard = item.value_mut();

                        let events = std::mem::take(guard);

                        events
                            .into_iter()
                            .map(|event| {
                                let CorrelationEventOccurrence {
                                    event,
                                    location,
                                    date,
                                } = event;

                                MetricCorrelationReading {
                                    time: date,
                                    host: host_name.clone(),
                                    extra: extra.clone(),
                                    correlation_id: corr_id.to_string(),
                                    location: location.to_string(),
                                    event: format!("{event:?}"),
                                }
                                .into_query(metric_name.clone())
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect(),
                MetricData::CorrelationDurationTracker(tracker) => {
                    let mut our_time = time;

                    let vec: MaybeVec<WriteQuery> = tracker
                        .accumulated
                        .iter_mut()
                        .map(|item| {
                            let corr_id = item.key().clone();

                            our_time += Duration::from_nanos(1);

                            MetricCorrelationTimeReading {
                                time: our_time,
                                host: host_name.clone(),
                                extra: extra.clone(),
                                correlation_id: corr_id.to_string(),
                                value: item.value().as_nanos() as i64,
                            }
                            .into_query(metric_name.clone())
                        })
                        .collect();

                    //info!("Correlation Duration Tracker: {:?} {:?}", vec.len(), vec);

                    vec
                }
                MetricData::CorrelationAggrDurationTracker(tracker) => {
                    let mut duration = tracker
                        .accumulated
                        .iter()
                        .map(|item| item.value().as_nanos())
                        .reduce(|tracked, tracked_2| tracked + tracked_2)
                        .unwrap_or(0);

                    duration /= std::cmp::max(1, tracker.accumulated.len() as u128);

                    MaybeVec::from_one(
                        MetricCorrelationAggrTimeReading {
                            time,
                            host: host_name.clone(),
                            extra: extra.clone(),
                            value: duration as i64,
                        }
                        .into_query(metric_name),
                    )
                }
                MetricData::CountMax(mut counts) => {
                    let max = if let MetricKind::CountMax(max) = metric.metric_type() {
                        *max
                    } else {
                        error!(
                            "Metric is not a CounterMax, but a {:?}",
                            metric.metric_type()
                        );
                        continue;
                    };

                    if counts.is_empty() {
                        continue;
                    }

                    counts.sort_by(|(a, _), (b, _)| a.cmp(b).reverse());

                    counts
                        .into_iter()
                        .take(max)
                        .map(|(count, time)| {
                            MetricCountMaxReading {
                                time,
                                host: host_name.clone(),
                                extra: extra.clone(),
                                value: count as i64,
                            }
                            .into_query(metric_name.clone())
                        })
                        .collect()
                }
                MetricData::CounterCorrelation(data) => {
                    let mut time = time;

                    data.counter_track
                        .iter_mut()
                        .map(|counter| {
                            time += Duration::from_nanos(1);

                            let correlation_id = counter.key().clone();
                            let value = counter.value().swap(0, Ordering::Relaxed);

                            MetricCorrelationCounterReading {
                                time,
                                host: host_name.clone(),
                                extra: extra.clone(),
                                correlation_id: correlation_id.to_string(),
                                value: value as i64,
                            }
                            .into_query(metric_name.clone())
                        })
                        .collect()
                }
            };

            match query {
                MaybeVec::None => {}
                MaybeVec::One(query) => readings.push(query),
                MaybeVec::Mult(mut queries) => readings.append(&mut queries),
            }
        }

        let instant = std::time::Instant::now();

        let readings_to_write = readings.len();

        let result: anyhow::Result<String> = readings
            .chunks(10000)
            .zip(iter::repeat_with(|| client.clone()))
            .try_fold(String::new(), |mut acc, (chunk, client)| {
                let result = rt::block_on(client.query(chunk.to_vec()));

                match result {
                    Ok(res) => {
                        acc.push_str(&res);
                        Ok(acc)
                    }
                    Err(e) => Err(anyhow::Error::new(e)),
                }
            });

        let time_taken = instant.elapsed();

        debug!(
            "Result of writing metrics: {:?} in {:?}. Wrote {} metrics",
            result, time_taken, readings_to_write
        );

        std::thread::sleep(Duration::from_millis(std::cmp::max(
            0,
            1000 - time_taken.as_millis() as u64,
        )));
    }
}
