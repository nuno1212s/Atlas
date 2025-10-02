# Atlas-Metrics

<div align="center">
  <h1>📊 Atlas Performance Metrics Collection</h1>
  <p><em>A comprehensive metrics collection and monitoring system for the Atlas Byzantine Fault Tolerant framework with InfluxDB integration and OS monitoring</em></p>

  [![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
</div>

---

## 📋 Table of Contents

- [Overview](#-overview)
- [Core Architecture](#-core-architecture)
- [Metric Types](#-metric-types)
- [Correlation Tracking](#-correlation-tracking)
- [OS Monitoring](#-os-monitoring)
- [InfluxDB Integration](#-influxdb-integration)
- [Configuration](#-configuration)
- [Usage Examples](#-usage-examples)
- [Performance Considerations](#-performance-considerations)

## 🏗️ Overview

Atlas-Metrics provides a **high-performance, thread-safe metrics collection system** specifically designed for distributed Byzantine Fault Tolerant systems. It features automatic InfluxDB integration, comprehensive OS monitoring, and advanced correlation tracking for complex distributed system analysis.

## 🏛️ Core Architecture

### Metrics Registry System

The system uses a global, thread-safe registry that manages multiple metric types with configurable concurrency levels:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Atlas-Metrics System                        │
└───────────────────────┬─────────────────────────────────────────┘
                        │
         ┌──────────────▼──────────────┐
         │   Global Metrics Registry   │
         │    (Thread-Safe Access)     │
         └──┬────────────────────────┬──┘
            │                        │
┌───────────▼─────────────┐ ┌───────▼──────────────┐
│    Metric Collection    │ │   Background Threads │
│ ┌─────────────────────┐ │ │ ┌──────────────────┐ │
│ │ Duration Metrics    │ │ │ │ InfluxDB Writer  │ │
│ │ (Welford's Method)  │ │ │ │ Thread           │ │
│ └─────────────────────┘ │ │ └──────────────────┘ │
│ ┌─────────────────────┐ │ │ ��──────────────────┐ │
│ │ Counter Metrics     │ │ │ │ OS Monitoring    │ │
│ │ (Atomic Operations) │ │ │ │ Thread           │ │
│ └─────────────────────┘ │ │ └──────────────────┘ │
│ ┌─────────────────────┐ │ └──────────────────────┘
│ │ Correlation         │ │
│ │ Tracking System     │ │
│ └─────────────────────┘ │
└─────────────────────────┘
            │
┌───────────▼──────���──────┐
│      InfluxDB Output    │
│ ┌─────────────────────┐ │
│ │ Time-Series Data    │ │
│ │ Storage             │ │
│ └─────────────────────┘ │
│ ┌─────────────────────┐ │
│ │ Batched Writes      │ │
│ │ (10k chunks)        │ │
│ └─────────────────────┘ │
└─────────────────────────┘
```

## 📊 Metric Types

### Core Metric Implementations

The system supports several specialized metric types, each optimized for different use cases:

#### Duration Metrics
```rust
pub enum MetricKind {
    Duration,                           // Statistical duration tracking
    Count,                              // Independent count averaging
    Counter,                            // Monotonic incremental counters
    CountMax(usize),                    // Top-N maximum value tracking
    Correlation,                        // Event correlation tracking
    CorrelationDurationTracker,         // Individual correlation timing
    CorrelationAggrDurationTracker,     // Aggregated correlation timing
    CounterCorrelation,                 // Counter with correlation IDs
}
```

#### Statistical Processing
Duration and Count metrics use **Welford's method** for online calculation of rolling statistics:
- **Mean**: Rolling average of observed values
- **Standard Deviation**: Computed using numerically stable algorithm
- **Count**: Total number of observations
- **Sum**: Total sum for additional analysis

### Thread-Safe Metric Data Storage

```rust
enum SafeMetricData {
    Sequential(Vec<Mutex<MetricData>>, ThreadLocal<Cell<usize>>),  // Round-robin per-thread buckets
    ThreadSafe(RwLock<MetricData>),                               // Reader-writer lock for correlation data
    Atomic(MetricData),                                           // Lock-free atomic operations
}
```

## 🔗 Correlation Tracking

### Advanced Event Correlation

The system provides sophisticated correlation tracking for distributed system analysis:

```rust
pub enum CorrelationEvent {
    Initialized,                        // Correlation ID created
    Passed,                            // ID passed between components
    Encapsulated(usize, Arc<str>),     // ID wrapped by another correlation
    Decapsulated,                      // ID unwrapped from encapsulation
    Ended,                             // Correlation lifecycle completed
}
```

### Correlation Features

- **Event Lifecycle Tracking**: Complete tracing of correlation IDs through system components
- **Encapsulation Support**: Nested correlation tracking for complex request flows
- **Duration Correlation**: Per-correlation-ID timing measurements
- **Counter Correlation**: Per-correlation-ID counter tracking
- **Aggregated Analysis**: Statistical aggregation of correlation timing data

## 🖥️ OS Monitoring

### System Resource Monitoring

Atlas-Metrics includes comprehensive OS monitoring with the following metrics:

#### CPU Monitoring
```rust
pub const OS_CPU_USER: &str = "OS_CPU_USER";
```
- **Per-core CPU usage**: Individual CPU core utilization tracking
- **Process-specific metrics**: Current process CPU consumption
- **Real-time updates**: 250ms refresh intervals

#### Memory Monitoring  
```rust
pub const OS_RAM_USAGE: &str = "OS_RAM_USAGE";
```
- **Process memory usage**: RSS memory consumption tracking
- **Real-time monitoring**: Continuous memory usage updates

#### Network Monitoring
```rust
pub const OS_NETWORK_UP: &str = "OS_NETWORK_UP";
pub const OS_NETWORK_DOWN: &str = "OS_NETWORK_DOWN";
```
- **Network throughput**: Bytes transmitted and received
- **Multi-interface aggregation**: Combined statistics across all network interfaces

## 🔌 InfluxDB Integration

### Automatic Time-Series Export

The system automatically exports all metrics to InfluxDB with structured data models:

#### Metric Export Types
```rust
#[derive(InfluxDbWriteable)]
pub struct MetricCounterReading {
    time: DateTime<Utc>,
    #[influxdb(tag)] host: String,
    #[influxdb(tag)] extra: String,
    value: i64,
}

#[derive(InfluxDbWriteable)]
pub struct MetricDurationReading {
    time: DateTime<Utc>,
    #[influxdb(tag)] host: String,
    #[influxdb(tag)] extra: String,
    value: f64,          // Statistical mean
    std_dev: f64,        // Standard deviation
}
```

#### Export Features
- **Batched Writes**: 10,000 metrics per batch for optimal performance
- **Tagged Data**: Host and experiment tagging for multi-node analysis
- **Correlation Export**: Detailed correlation event and timing data
- **1-Second Intervals**: Regular metric collection and export

## ⚙️ Configuration

### Metrics Initialization

```rust
#[derive(Debug, Clone)]
pub struct InfluxDBArgs {
    pub ip: String,              // InfluxDB server address
    pub db_name: String,         // Database name
    pub user: String,            // Authentication username
    pub password: String,        // Authentication password
    pub node_id: NodeId,         // Unique node identifier
    pub extra: Option<String>,   // Experiment/run identifier
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq)]
pub enum MetricLevel {
    Disabled,    // No metric collection
    Trace,       // Detailed tracing metrics
    Debug,       // Debug-level metrics
    Info,        // Standard operational metrics
}
```

### Registry Configuration

```rust
pub struct MetricRegistryInfo {
    index: usize,                           // Unique metric index
    name: String,                           // Metric name for InfluxDB
    kind: MetricKind,                       // Type of metric
    level: MetricLevel,                     // Minimum level for collection
    concurrency_override: Option<usize>,    // Custom thread bucket count
}
```

## 🚀 Usage Examples

### Basic Metrics Setup

```rust
use atlas_metrics::{initialize_metrics, with_metrics, with_concurrency, with_metric_level};
use atlas_metrics::{InfluxDBArgs, MetricLevel, MetricRegistry, MetricKind};
use atlas_common::node_id::NodeId;

// Define metrics registry
let metrics = vec![
    MetricRegistry::from((0, "request_duration".to_string(), MetricKind::Duration)),
    MetricRegistry::from((1, "requests_total".to_string(), MetricKind::Counter)),
    MetricRegistry::from((2, "active_connections".to_string(), MetricKind::Count)),
    MetricRegistry::from((3, "correlation_tracker".to_string(), MetricKind::Correlation)),
];

// Configure InfluxDB connection
let influx_config = InfluxDBArgs::new(
    "http://localhost:8086".to_string(),
    "atlas_metrics".to_string(),
    "admin".to_string(),
    "password".to_string(),
    NodeId::from(0),
    Some("experiment_1".to_string()),
);

// Initialize metrics system
initialize_metrics(
    vec![
        with_metrics(metrics),
        with_concurrency(4),              // 4 thread buckets for sequential metrics
        with_metric_level(MetricLevel::Info),
    ],
    influx_config,
);
```

### Recording Metrics

```rust
use atlas_metrics::metrics::{
    metric_duration_start, metric_duration_end, metric_increment,
    metric_store_count, metric_local_duration_start, metric_local_duration_end
};

const REQUEST_DURATION_METRIC: usize = 0;
const REQUESTS_TOTAL_METRIC: usize = 1;
const BATCH_SIZE_METRIC: usize = 2;

// Duration measurement with start/end
metric_duration_start(REQUEST_DURATION_METRIC);
// ... perform operation ...
metric_duration_end(REQUEST_DURATION_METRIC);

// Local scope duration measurement
let start = metric_local_duration_start();
// ... perform operation ...
metric_local_duration_end(REQUEST_DURATION_METRIC, start);

// Counter increment
metric_increment(REQUESTS_TOTAL_METRIC, Some(1));

// Count averaging (e.g., batch sizes)
metric_store_count(BATCH_SIZE_METRIC, batch.len());
```

### Correlation Tracking

```rust
use atlas_metrics::metrics::{
    metric_initialize_correlation_id, metric_correlation_id_passed,
    metric_correlation_id_ended, metric_correlation_time_start, metric_correlation_time_end
};
use std::sync::Arc;

const CORRELATION_METRIC: usize = 3;
const CORRELATION_TIME_METRIC: usize = 4;

// Initialize correlation tracking
let correlation_id = "req_12345";
let location = Arc::from("client_handler");
metric_initialize_correlation_id(CORRELATION_METRIC, correlation_id, location.clone());

// Start timing for this correlation
metric_correlation_time_start(CORRELATION_TIME_METRIC, correlation_id);

// Track correlation passing through system components
let consensus_location = Arc::from("consensus_module");
metric_correlation_id_passed(CORRELATION_METRIC, correlation_id, consensus_location);

// End timing measurement
metric_correlation_time_end(CORRELATION_TIME_METRIC, correlation_id);

// Mark correlation as completed
let completion_location = Arc::from("response_handler");
metric_correlation_id_ended(CORRELATION_METRIC, correlation_id, completion_location);
```

### Advanced Configuration

```rust
// Custom concurrency override for high-contention metrics
let high_throughput_metric = MetricRegistry::from((
    10, 
    "high_throughput_counter".to_string(), 
    MetricKind::Counter, 
    MetricLevel::Info, 
    8  // 8 thread buckets instead of default
));

// Maximum value tracking
let top_latencies = MetricRegistry::from((
    11, 
    "top_request_latencies".to_string(), 
    MetricKind::CountMax(10)  // Track top 10 values
));
```

## 📊 Performance Considerations

### Optimization Features

- **Lock-free Operations**: Atomic operations for counters and duration statistics
- **Thread-local Storage**: Round-robin thread buckets minimize contention
- **Batched InfluxDB Writes**: 10,000 metric chunks reduce network overhead
- **Welford's Algorithm**: Numerically stable online statistics calculation
- **Configurable Concurrency**: Custom thread bucket sizing per metric

### Resource Usage

```rust
// Configurable parameters for tuning
const DEFAULT_CONCURRENCY: usize = 2;           // Thread buckets per metric
const INFLUX_BATCH_SIZE: usize = 10_000;        // Metrics per InfluxDB write
const METRICS_EXPORT_INTERVAL: u64 = 1000;     // Milliseconds between exports
const OS_MONITORING_INTERVAL: u64 = 250;       // OS metrics refresh rate
```

### Memory Management

- **Efficient Correlation Storage**: DashMap for concurrent correlation tracking
- **Atomic Metric Data**: Minimal memory overhead for frequently updated metrics
- **Bounded Collection**: Automatic metric data collection prevents memory leaks

## 🔧 Integration with Atlas Framework

Atlas-Metrics is specifically designed for integration with Atlas BFT components:

- **Consensus Protocol Metrics**: Track consensus round timing and message counts
- **Network Layer Metrics**: Monitor communication performance and throughput
- **Client Request Metrics**: Measure end-to-end request processing
- **State Transfer Metrics**: Track state synchronization performance
- **Execution Engine Metrics**: Monitor application execution timing

The system provides metric-level configuration to enable appropriate monitoring for production vs. development environments, with minimal performance impact on the critical consensus path.

## 📄 License

This module is licensed under the MIT License - see the LICENSE.txt file for details.

---

Atlas-Metrics provides the essential observability infrastructure for Atlas Byzantine Fault Tolerant systems, enabling performance analysis, system debugging, and production monitoring through comprehensive metrics collection and automated InfluxDB integration.
