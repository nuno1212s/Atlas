# Atlas-SMR-Application

<div align="center">
  <h1>🚀 Atlas SMR Application Framework</h1>
  <p><em>Core abstractions and interfaces for building Byzantine Fault Tolerant State Machine Replication applications on the Atlas framework</em></p>

  [![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
</div>

---

## 📋 Table of Contents

- [Core Abstractions](#-core-abstractions)
- [Request Processing](#-request-processing)
- [State Management](#-state-management)
- [Execution Framework](#-execution-framework)
- [Integration with Atlas-SMR-Core](#-integration-with-atlas-smr-core)
- [Building Your Application](#-building-your-application)
- [Performance Considerations](#-performance-considerations)
- [Advanced Features](#-advanced-features)
- [Dependencies](#-dependencies)

## 🏗️ Core Abstractions

Atlas-SMR-Application defines the fundamental interfaces that application developers must implement to build Byzantine Fault Tolerant (BFT) State Machine Replication applications using the Atlas framework. This module serves as the bridge between user applications and the Atlas-SMR-Core consensus infrastructure.

### Application Interface

The central interface for SMR applications is the `Application` trait, which defines how requests are processed and state is managed:

```rust
pub trait Application<S>: Send + Sync {
    type AppData: ApplicationData + 'static;

    /// Returns the initial state of the application
    fn initial_state() -> Result<S>;

    /// Process an unordered client request without altering state
    fn unordered_execution(&self, state: &S, request: Request<Self, S>) -> Reply<Self, S>;

    /// Process an unordered batch of requests (optional optimization)
    fn unordered_batched_execution(
        &self,
        state: &S,
        requests: UnorderedBatch<Request<Self, S>>,
    ) -> BatchReplies<Reply<Self, S>>;

    /// Process a state-modifying request and update application state
    fn update(&self, state: &mut S, request: Request<Self, S>) -> Reply<Self, S>;

    /// Process a batch of state-modifying requests (optional optimization)
    fn update_batch(
        &self,
        state: &mut S,
        batch: UpdateBatch<Request<Self, S>>,
    ) -> BatchReplies<Reply<Self, S>>;
}
```

Key characteristics:
- **Thread Safety**: Applications must be `Send + Sync` for multi-threaded execution
- **Concurrency Control**: State management handles synchronization, not the application
- **Batch Processing**: Optional batch methods for performance optimization
- **Dual Execution Models**: Support for both ordered and unordered request processing

### Application Data Types

Applications define their communication types through the `ApplicationData` trait:

```rust
pub trait ApplicationData: Send + Sync {
    /// Request type sent by clients to replicas
    type Request: SerMsg + Sync + 'static;

    /// Reply type sent by replicas to clients  
    type Reply: SerMsg + Sync + 'static;

    // Serialization methods for network communication and persistence
    fn serialize_request<W: Write>(w: W, request: &Self::Request) -> Result<()>;
    fn deserialize_request<R: Read>(r: R) -> Result<Self::Request>;
    fn serialize_reply<W: Write>(w: W, reply: &Self::Reply) -> Result<()>;
    fn deserialize_reply<R: Read>(r: R) -> Result<Self::Reply>;
}
```

## 🔄 Request Processing

### Request and Reply Types

The framework provides strongly-typed request and reply abstractions:

```rust
// Convenience type aliases based on application
pub type Request<A, S> = <<A as Application<S>>::AppData as ApplicationData>::Request;
pub type Reply<A, S> = <<A as Application<S>>::AppData as ApplicationData>::Reply;
```

### Update Processing

Individual requests are wrapped in `Update` structures containing client metadata:

```rust
pub struct Update<O> {
    from: NodeId,           // Client identifier
    session_id: SeqNo,      // Session sequence number
    operation_id: SeqNo,    // Operation sequence number  
    operation: O,           // The actual request
}
```

### Batch Processing

#### Ordered Batches

For state-modifying operations that require total ordering:

```rust
pub struct UpdateBatch<O> {
    seq_no: SeqNo,                    // Consensus sequence number
    inner: Vec<Update<O>>,            // Batch of updates
    meta: Option<BatchMeta>,          // Performance metrics
}
```

Features:
- **Consensus Ordering**: Each batch has a sequence number from consensus
- **Metrics Integration**: Optional performance tracking
- **Orderable**: Implements `Orderable` trait for Atlas-Core integration

#### Unordered Batches

For read-only operations that bypass consensus:

```rust
pub struct UnorderedBatch<O> {
    inner: Vec<Update<O>>,   // Batch of read-only requests
}
```

#### Reply Batches

Structured responses for batch processing:

```rust
pub struct BatchReplies<P> {
    inner: Vec<UpdateReply<P>>,
}

pub struct UpdateReply<P> {
    to: NodeId,              // Target client
    session_id: SeqNo,       // Session identifier
    operation_id: SeqNo,     // Operation identifier
    payload: P,              // Reply payload
}
```

## 🗂️ State Management

Atlas-SMR-Application supports two distinct state management models:

### Monolithic State

For applications with atomic, indivisible state:

```rust
pub trait MonolithicState: NonSyncSerMsg {
    fn serialize_state<W: Write>(w: W, state: &Self) -> Result<()>;
    fn deserialize_state<R: Read>(r: R) -> Result<Self> where Self: Sized;
}
```

#### State Messages

```rust
pub struct AppStateMessage<S: MonolithicState> {
    seq: SeqNo,    // Sequence number of checkpoint
    state: S,      // Complete application state
}

pub struct InstallStateMessage<S: MonolithicState> {
    state: S,      // State to install during recovery
}
```

#### State Integrity

```rust
// Built-in state digest computation for integrity verification
pub fn digest_state<S: MonolithicState>(appstate: &S) -> Result<Digest>
```

### Divisible State

For applications with partitionable state that can be transferred incrementally:

```rust
pub trait DivisibleState: Sized + Send {
    type PartDescription: PartId + SerMsg;           // Part identifier
    type StateDescriptor: DivisibleStateDescriptor<Self> + SerMsg;  // Complete state description
    type StatePart: StatePart<Self> + SerMsg;        // Individual state part

    /// Get current state description
    fn get_descriptor(&self) -> &Self::StateDescriptor;
    
    /// Accept new parts into current state
    fn accept_parts(&mut self, parts: Vec<Self::StatePart>) -> Result<()>;
    
    /// Prepare a checkpoint of current state
    fn prepare_checkpoint(&mut self) -> Result<&Self::StateDescriptor>;
    
    /// Extract specific parts for transfer
    fn get_parts(&self, parts: &[Self::PartDescription]) -> Result<Vec<Self::StatePart>>;
}
```

#### State Descriptors

```rust
pub trait DivisibleStateDescriptor<S: DivisibleState>: Orderable + PartialEq + Clone + Send {
    /// Get all part descriptions in this state
    fn parts(&self) -> &Vec<S::PartDescription>;
    
    /// Compare with another descriptor to find missing parts
    fn compare_descriptors(&self, other: &Self) -> Vec<S::PartDescription>;
}
```

#### State Transfer Messages

```rust
pub enum InstallStateMessage<S: DivisibleState> {
    StateDescriptor(S::StateDescriptor),    // State metadata
    StatePart(MaybeVec<S::StatePart>),     // Individual parts
    Done,                                   // Transfer complete
}

pub enum AppState<S: DivisibleState> {
    StateDescriptor(S::StateDescriptor),    // Checkpoint descriptor
    StatePart(MaybeVec<S::StatePart>),     // Parts for transfer
    Done,                                   // Checkpoint complete
}
```

## ⚙️ Execution Framework

### Execution Handle

The `ExecutorHandle` serves as the interface between the consensus layer and application execution:

```rust
pub struct ExecutorHandle<RQ> {
    e_tx: ChannelSyncTx<ExecutionRequest<RQ>>,
}
```

#### Execution Operations

```rust
impl<RQ> ExecutorHandle<RQ> {
    /// Trigger state channel polling
    pub fn poll_state_channel(&self) -> Result<()>;
    
    /// Catch up execution with consensus decisions
    pub fn catch_up_to_quorum(&self, requests: MaybeVec<UpdateBatch<RQ>>) -> Result<()>;
    
    /// Queue ordered batch for execution
    pub fn queue_update(&self, batch: UpdateBatch<RQ>) -> Result<()>;
    
    /// Queue unordered requests for execution
    pub fn queue_update_unordered(&self, requests: UnorderedBatch<RQ>) -> Result<()>;
    
    /// Queue batch and trigger checkpoint
    pub fn queue_update_and_get_appstate(&self, batch: UpdateBatch<RQ>) -> Result<()>;
}
```

### Execution Request Types

```rust
pub enum ExecutionRequest<O> {
    PollStateChannel,                                    // State synchronization
    CatchUp(MaybeVec<UpdateBatch<O>>),                  // Recovery processing  
    Update((UpdateBatch<O>, Instant)),                   // Standard execution
    UpdateAndGetAppstate((UpdateBatch<O>, Instant)),     // Checkpoint execution
    ExecuteUnordered(UnorderedBatch<O>),                // Read-only execution
    Read(NodeId),                                        // State read request
}
```

## 🔗 Integration with Atlas-SMR-Core

Atlas-SMR-Application integrates seamlessly with Atlas-SMR-Core through several key interfaces:

### Execution Integration

- **Request Processing**: `ExecutorHandle` receives `UpdateBatch` and `UnorderedBatch` from Atlas-SMR-Core's `WrappedExecHandle`
- **State Management**: Application state interfaces with Atlas-SMR-Core's state transfer protocols
- **Reply Handling**: `BatchReplies` integrate with Atlas-SMR-Core's `ReplyNode` abstraction

### Message Flow

```
Atlas-SMR-Core Request Pre-Processing → UpdateBatch/UnorderedBatch → ExecutorHandle
                    ↓
Application::update_batch() / Application::unordered_batched_execution()
                    ↓  
BatchReplies → Atlas-SMR-Core ReplyNode → Client Responses
```

### Serialization Integration

- **Network Communication**: `ApplicationData` serialization methods integrate with Atlas-Communication
- **State Persistence**: State serialization works with Atlas-SMR-Core's persistent logging
- **Message Types**: All types implement `SerMsg` for Atlas framework compatibility

## 🚀 Building Your Application

### Step 1: Define Application Data

```rust
use atlas_smr_application::serialize::ApplicationData;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub enum CalculatorRequest {
    Add(i64),
    Subtract(i64),
    Multiply(i64),
    Divide(i64),
    Get,
    Clear,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum CalculatorReply {
    Value(i64),
    Error(String),
    Ok,
}

pub struct CalculatorData;

impl ApplicationData for CalculatorData {
    type Request = CalculatorRequest;
    type Reply = CalculatorReply;

    fn serialize_request<W: Write>(mut w: W, request: &Self::Request) -> Result<()> {
        bincode::serialize_into(&mut w, request)?;
        Ok(())
    }

    fn deserialize_request<R: Read>(mut r: R) -> Result<Self::Request> {
        Ok(bincode::deserialize_from(&mut r)?)
    }

    fn serialize_reply<W: Write>(mut w: W, reply: &Self::Reply) -> Result<()> {
        bincode::serialize_into(&mut w, reply)?;
        Ok(())
    }

    fn deserialize_reply<R: Read>(mut r: R) -> Result<Self::Reply> {
        Ok(bincode::deserialize_from(&mut r)?)
    }
}
```

### Step 2: Define Application State

```rust
#[derive(Clone)]
pub struct CalculatorState {
    current_value: i64,
}

// For monolithic state:
impl MonolithicState for CalculatorState {
    fn serialize_state<W: Write>(mut w: W, state: &Self) -> Result<()> {
        bincode::serialize_into(&mut w, state)?;
        Ok(())
    }

    fn deserialize_state<R: Read>(mut r: R) -> Result<Self> {
        Ok(bincode::deserialize_from(&mut r)?)
    }
}
```

### Step 3: Implement Application Logic

```rust
use atlas_smr_application::app::{Application, BatchReplies, UpdateBatch, UnorderedBatch};

pub struct Calculator;

impl Application<CalculatorState> for Calculator {
    type AppData = CalculatorData;

    fn initial_state() -> Result<CalculatorState> {
        Ok(CalculatorState { current_value: 0 })
    }

    fn unordered_execution(
        &self, 
        state: &CalculatorState, 
        request: CalculatorRequest
    ) -> CalculatorReply {
        match request {
            CalculatorRequest::Get => CalculatorReply::Value(state.current_value),
            _ => CalculatorReply::Error("Only Get supported for unordered execution".to_string()),
        }
    }

    fn update(
        &self, 
        state: &mut CalculatorState, 
        request: CalculatorRequest
    ) -> CalculatorReply {
        match request {
            CalculatorRequest::Add(val) => {
                state.current_value += val;
                CalculatorReply::Value(state.current_value)
            }
            CalculatorRequest::Subtract(val) => {
                state.current_value -= val;
                CalculatorReply::Value(state.current_value)
            }
            CalculatorRequest::Multiply(val) => {
                state.current_value *= val;
                CalculatorReply::Value(state.current_value)
            }
            CalculatorRequest::Divide(val) => {
                if val == 0 {
                    CalculatorReply::Error("Division by zero".to_string())
                } else {
                    state.current_value /= val;
                    CalculatorReply::Value(state.current_value)
                }
            }
            CalculatorRequest::Clear => {
                state.current_value = 0;
                CalculatorReply::Ok
            }
            CalculatorRequest::Get => CalculatorReply::Value(state.current_value),
        }
    }

    // Optional: Implement batch processing for better performance
    fn update_batch(
        &self,
        state: &mut CalculatorState,
        batch: UpdateBatch<CalculatorRequest>,
    ) -> BatchReplies<CalculatorReply> {
        let mut replies = BatchReplies::with_capacity(batch.len());
        
        for update in batch.into_inner() {
            let (peer_id, sess, op_id, req) = update.into_inner();
            let reply = self.update(state, req);
            replies.add(peer_id, sess, op_id, reply);
        }
        
        replies
    }
}
```

### Step 4: Integration with Atlas-SMR-Core

Your application integrates with Atlas-SMR-Core through:

1. **Execution Layer**: Atlas-SMR-Core's `WrappedExecHandle` transforms consensus decisions into `ExecutorHandle` calls
2. **Networking**: Atlas-SMR-Core's networking layer handles message routing using your `ApplicationData` serialization
3. **State Transfer**: Atlas-SMR-Core's state transfer protocols use your state management abstractions

## 📈 Performance Considerations

### Batch Processing
- Implement `update_batch()` and `unordered_batched_execution()` for better throughput
- Use appropriate batch sizes based on your application's characteristics

### State Design
- **Monolithic State**: Simpler but requires full state transfer during recovery
- **Divisible State**: More complex but enables incremental state transfer and better scalability

### Serialization
- Choose efficient serialization formats (e.g., bincode, Cap'n Proto)
- Consider message size impact on network performance

### Concurrency
- Applications must be thread-safe (`Send + Sync`)
- State synchronization is handled by the execution framework
- Avoid blocking operations in application methods

## 🛠️ Advanced Features

### State Checkpointing
The framework supports automatic state checkpointing through:
- `ExecutorHandle::queue_update_and_get_appstate()` for checkpoint triggers
- State serialization for persistence and transfer
- Digest computation for integrity verification

### Request Type Optimization
- **Unordered Requests**: Bypass consensus for read-only operations
- **Ordered Requests**: Go through consensus for state modifications
- **Batch Processing**: Process multiple requests efficiently

### Error Handling
All operations return `Result<T>` types with proper error propagation through the `anyhow` error handling framework.

## 📦 Dependencies

- **atlas-common**: Core utilities and data structures
- **atlas-communication**: Network communication abstractions  
- **atlas-metrics**: Performance monitoring integration
- **anyhow**: Error handling
- **thiserror**: Error type derivation

## 📄 License

This module is licensed under the MIT License - see the LICENSE.txt file for details.
