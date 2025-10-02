# Atlas-SMR-Execution

<div align="center">
  <h1>⚡ Atlas SMR Execution Framework</h1>
  <p><em>High-performance execution layer for State Machine Replication applications with support for both single-threaded and multi-threaded scalable execution models</em></p>

  [![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
</div>

---

## 📋 Table of Contents

- [Core Architecture](#-core-architecture)
- [Execution Types](#-execution-types)
- [State Management Models](#-state-management-models)
- [Scalable Execution Framework](#-scalable-execution-framework)
- [Performance Optimizations](#-performance-optimizations)
- [Integration with Atlas-SMR-Core](#-integration-with-atlas-smr-core)
- [Metrics and Monitoring](#-metrics-and-monitoring)
- [Usage Guide](#-usage-guide)
- [Advanced Features](#-advanced-features)

## 🏗️ Core Architecture

Atlas-SMR-Execution serves as the execution layer that bridges Atlas-SMR-Core's consensus decisions with Atlas-SMR-Application's business logic. It provides multiple execution strategies optimized for different application characteristics and performance requirements.

### Execution Layer Positioning

```
Atlas-SMR-Core (Consensus) → Atlas-SMR-Execution → Atlas-SMR-Application (Business Logic)
                ↕                      ↕                        ↕
        Decision Processing    Execution Orchestration    State Management
```

The execution framework receives `UpdateBatch` and `UnorderedBatch` requests from Atlas-SMR-Core's `WrappedExecHandle` and coordinates their execution against application state, returning `BatchReplies` for client responses.

## ⚙️ Execution Types

Atlas-SMR-Execution provides four distinct execution models, each optimized for specific application and state characteristics:

### 1. Single-Threaded Monolithic Executor

**Type**: `SingleThreadedMonExecutor`  
**Best For**: Simple applications with atomic state and moderate throughput requirements

```rust
impl<A, S, NT> TMonolithicStateExecutor<A, S, NT> for SingleThreadedMonExecutor
where
    A: Application<S> + 'static,
    S: MonolithicState + 'static,
    NT: 'static,
```

**Characteristics**:
- **Sequential Execution**: All operations execute in strict order
- **Simple State Model**: Works with `MonolithicState` applications
- **Low Overhead**: Minimal coordination complexity
- **Memory Efficient**: Single execution thread with direct state access

### 2. Multi-Threaded Monolithic Executor

**Type**: `MultiThreadedMonExecutor`  
**Best For**: Applications with atomic state requiring high throughput through parallel execution

```rust
impl<A, S, NT> TMonolithicStateExecutor<A, S, NT> for MultiThreadedMonExecutor
where
    A: ScalableApp<S> + 'static,
    S: MonolithicState + CRUDState + Send + Sync + 'static,
    NT: 'static,
```

**Characteristics**:
- **Speculative Parallel Execution**: Operations execute concurrently with collision detection
- **CRUD State Interface**: Requires `CRUDState` implementation for concurrent access
- **Collision Recovery**: Conflicting operations re-execute sequentially
- **Thread Pool Utilization**: Configurable thread pool for parallel processing

### 3. Single-Threaded Divisible Executor

**Type**: `SingleThreadedDivExecutor`  
**Best For**: Applications with partitionable state and moderate throughput needs

```rust
impl<A, S, NT> TDivisibleStateExecutor<A, S, NT> for SingleThreadedDivExecutor
where
    A: Application<S> + 'static,
    S: DivisibleState + Send + 'static,
    NT: 'static,
```

**Characteristics**:
- **Sequential Execution**: Operations execute in order
- **Partitionable State**: Supports `DivisibleState` for incremental state transfer
- **State Part Management**: Handles state descriptors and individual state parts
- **Transfer Optimization**: Enables efficient state synchronization

### 4. Multi-Threaded Divisible Executor

**Type**: `MultiThreadedDivExecutor`  
**Best For**: High-throughput applications with partitionable state requiring maximum parallelization

```rust
impl<A, S, NT> TDivisibleStateExecutor<A, S, NT> for MultiThreadedDivExecutor
where
    A: ScalableApp<S> + Send + 'static,
    S: DivisibleState + CRUDState + Send + Sync + 'static,
    NT: 'static,
```

**Characteristics**:
- **Speculative Parallel Execution**: Full parallelization with collision detection
- **Advanced State Management**: Combines `DivisibleState` and `CRUDState` capabilities
- **Maximum Throughput**: Optimized for high-performance scenarios
- **Complex Coordination**: Sophisticated collision detection and recovery mechanisms

## 🗂️ State Management Models

### Monolithic State Model

For applications with atomic, indivisible state that must be transferred as a complete unit:

```rust
pub trait TMonolithicStateExecutor<A, S, NT>
where
    A: Application<S> + 'static,
    S: MonolithicState + 'static,
    NT: 'static,
{
    fn init(
        work_receiver: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<(
        ChannelSyncTx<InstallStateMessage<S>>,
        ChannelSyncRx<AppStateMessage<S>>,
    )>;
}
```

**Features**:
- **Complete State Checkpoints**: Entire state captured in checkpoints
- **Atomic State Transfer**: State transferred as single unit during recovery
- **Simplified Management**: Single state entity reduces coordination complexity

### Divisible State Model

For applications with partitionable state enabling incremental transfer and enhanced scalability:

```rust
pub trait TDivisibleStateExecutor<A, S, NT>
where
    A: Application<S> + 'static,
    S: DivisibleState + 'static,
    NT: 'static,
{
    fn init(
        work_receiver: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<(
        ChannelSyncTx<state::divisible_state::InstallStateMessage<S>>,
        ChannelSyncRx<state::divisible_state::AppStateMessage<S>>,
    )>;
}
```

**Features**:
- **Incremental State Transfer**: Individual state parts transferred independently
- **Efficient Synchronization**: Only missing parts transferred during recovery
- **Scalable Architecture**: Supports applications with large, partitioned datasets

## 🚀 Scalable Execution Framework

### Speculative Execution Model

The multi-threaded executors utilize a sophisticated speculative execution model:

#### Execution Units

```rust
pub struct ExecutionUnit<'a, S>
where
    S: CRUDState,
{
    seq_no: SeqNo,                           // Batch sequence number
    position_in_batch: usize,                // Operation position
    accesses: RefCell<Vec<Access>>,          // Data access tracking
    cache: HashMap<String, HashMap<Vec<u8>, Vec<u8>>>, // Local write cache
    state_reference: &'a S,                  // Read-only state reference
}
```

#### Collision Detection

The framework tracks data access patterns to identify conflicts:

```rust
#[derive(Copy, Clone, Debug, PartialOrd, PartialEq, Eq, Ord)]
enum AccessType {
    Read,    // Read access - can be parallelized
    Write,   // Write access - conflicts with any other access
    Delete,  // Delete access - conflicts with any other access
}
```

**Collision Rules**:
- **Write-Anything**: Write operations conflict with any other access to the same data
- **Delete-Anything**: Delete operations conflict with any other access to the same data  
- **Read-Read**: Multiple read operations can execute in parallel without conflict

#### Execution Process

1. **Parallel Execution**: Operations execute concurrently using `ExecutionUnit` abstractions
2. **Access Tracking**: All data accesses (read/write/delete) are recorded
3. **Collision Analysis**: Real-time collision detection as operations complete
4. **Conflict Resolution**: Conflicting operations re-execute sequentially in correct order
5. **State Application**: Successful speculative results apply to actual state

### CRUD State Interface

Multi-threaded execution requires applications to implement the `CRUDState` trait:

```rust
pub trait CRUDState: Send {
    fn read(&self, column: &str, key: &[u8]) -> Option<Vec<u8>>;
    fn create(&mut self, column: &str, key: &[u8], value: &[u8]) -> bool;
    fn update(&mut self, column: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>>;
    fn delete(&mut self, column: &str, key: &[u8]) -> Option<Vec<u8>>;
}
```

### Scalable Application Interface

Applications utilizing multi-threaded execution must implement `ScalableApp`:

```rust
pub trait ScalableApp<S>: Application<S> + Sync
where
    S: CRUDState,
{
    fn speculatively_execute(
        &self,
        state: &mut impl CRUDState,
        request: Request<Self, S>,
    ) -> Reply<Self, S>;
}
```

## ⚡ Performance Optimizations

### Thread Pool Configuration

```rust
const THREAD_POOL_THREADS: u32 = 4;  // Configurable thread pool size

// Thread pool initialization for scalable executors
ThreadPoolBuilder::new().num_threads(4).build()?
```

### Batch Processing Optimizations

#### Ordered Execution
- **Speculative Parallelization**: Operations execute concurrently when possible
- **Collision Recovery**: Conflicting operations re-execute sequentially
- **Chunk-Based Processing**: Work distributed across thread pool efficiently

#### Unordered Execution  
- **Full Parallelization**: Read-only operations execute without coordination
- **Rayon Integration**: Utilizes `par_iter()` for optimal parallel processing
- **No Collision Detection**: Maximum throughput for non-conflicting operations

### Memory Management

- **Local Caching**: `ExecutionUnit` caches intermediate results
- **Minimal State Copying**: Direct references avoid unnecessary data duplication
- **Efficient Data Structures**: `HashMap` and `BTreeMap` for optimal access patterns

## 🔗 Integration with Atlas-SMR-Core

### Request Processing Pipeline

```rust
// Atlas-SMR-Core provides execution requests
pub enum ExecutionRequest<O> {
    PollStateChannel,                          // State synchronization trigger
    CatchUp(MaybeVec<UpdateBatch<O>>),        // Recovery batch processing
    Update((UpdateBatch<O>, Instant)),         // Standard ordered execution
    UpdateAndGetAppstate((UpdateBatch<O>, Instant)), // Execution with checkpoint
    ExecuteUnordered(UnorderedBatch<O>),      // Read-only execution
    Read(NodeId),                             // State read request
}
```

### Execution Handle Integration

```rust
impl<RQ> ExecutorHandle<RQ> {
    pub fn catch_up_to_quorum(&self, requests: MaybeVec<UpdateBatch<RQ>>) -> Result<()>;
    pub fn queue_update(&self, batch: UpdateBatch<RQ>) -> Result<()>;
    pub fn queue_update_unordered(&self, requests: UnorderedBatch<RQ>) -> Result<()>;
    pub fn queue_update_and_get_appstate(&self, batch: UpdateBatch<RQ>) -> Result<()>;
}
```

### Reply Processing

The framework implements two reply strategies:

#### Replica Replier
```rust
pub struct ReplicaReplier;
// Sends replies for both ordered and unordered requests
```

#### Follower Replier
```rust
pub struct FollowerReplier;
// Only sends replies for unordered requests (followers don't participate in consensus)
```

## 📊 Metrics and Monitoring

Atlas-SMR-Execution provides comprehensive performance metrics:

### Execution Metrics

```rust
// Duration Metrics (800-806 series)
pub const EXECUTION_LATENCY_TIME_ID: usize = 800;        // End-to-end execution latency
pub const EXECUTION_TIME_TAKEN_ID: usize = 801;          // Batch execution duration  
pub const REPLIES_SENT_TIME_ID: usize = 802;             // Reply delivery time
pub const UNORDERED_EXECUTION_TIME_TAKEN_ID: usize = 806; // Unordered execution duration

// Throughput Metrics
pub const OPERATIONS_EXECUTED_PER_SECOND_ID: usize = 804; // Ordered operations throughput
pub const UNORDERED_OPS_PER_SECOND_ID: usize = 805;       // Unordered operations throughput
```

### Performance Tracking

All execution paths include automatic performance measurement:

```rust
let start = Instant::now();
let reply_batch = application.update_batch(state, batch);
metric_duration(EXECUTION_TIME_TAKEN_ID, start.elapsed());
metric_increment(OPERATIONS_EXECUTED_PER_SECOND_ID, Some(operations));
```

## 🛠️ Usage Guide

### Choosing the Right Executor

| Application Characteristics | Recommended Executor |
|----------------------------|---------------------|
| Simple state, moderate throughput | `SingleThreadedMonExecutor` |
| Complex state, high throughput | `MultiThreadedMonExecutor` |
| Partitioned state, moderate throughput | `SingleThreadedDivExecutor` |
| Partitioned state, maximum performance | `MultiThreadedDivExecutor` |

### Implementation Requirements

#### For Single-Threaded Execution
- Implement `Application<S>` trait from Atlas-SMR-Application
- Choose appropriate state model (`MonolithicState` or `DivisibleState`)

#### For Multi-Threaded Execution
- Implement both `Application<S>` and `ScalableApp<S>` traits
- Implement `CRUDState` trait for your state type
- Ensure state types are `Send + Sync`
- Implement `speculatively_execute()` method for parallel execution

### Configuration

```rust
// Buffer sizes for execution channels
const EXECUTING_BUFFER: usize = 16384;  // Main execution request buffer
const STATE_BUFFER: usize = 128;        // State management buffer

// Thread pool configuration
ThreadPoolBuilder::new().num_threads(4).build()?
```

## 🔧 Advanced Features

### State Checkpointing Integration

Executors integrate with Atlas-SMR-Core's state transfer protocols:

- **Monolithic Checkpoints**: Complete state snapshots with sequence numbers
- **Divisible Checkpoints**: State descriptors and individual parts for incremental transfer
- **Checkpoint Triggers**: Automatic checkpoint generation on `UpdateAndGetAppstate` requests

### Recovery and Catch-up

The execution framework supports sophisticated recovery mechanisms:

- **Batch Recovery**: `CatchUp` requests process multiple decision batches during recovery
- **State Installation**: Integration with state transfer protocols for state recovery
- **Request Replay**: Ability to replay requests from checkpoints

### Channel-Based Architecture

All communication utilizes high-performance bounded channels:

- **Work Distribution**: Execution requests distributed via dedicated channels
- **State Management**: State installation and checkpointing via separate channels
- **Backpressure Handling**: Bounded channels provide natural flow control

## 📦 Dependencies

- **atlas-smr-application**: Application interface and request/reply types
- **atlas-smr-core**: Integration with consensus and networking layers
- **atlas-common**: Shared utilities and data structures
- **atlas-metrics**: Performance monitoring and metrics collection
- **rayon**: High-performance parallel execution framework
- **anyhow**: Error handling and propagation

## 🎯 Design Principles

1. **Flexibility**: Multiple execution models for different application needs
2. **Performance**: Optimized parallel execution with collision detection
3. **Integration**: Seamless integration with Atlas consensus infrastructure
4. **Monitoring**: Comprehensive metrics for performance optimization
5. **Scalability**: Support for both small and large-scale applications

Atlas-SMR-Execution provides the critical execution layer that transforms consensus decisions into application state changes, offering multiple execution strategies optimized for different application characteristics while maintaining strong integration with the broader Atlas framework.
