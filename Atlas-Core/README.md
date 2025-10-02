# Atlas-Core

<div align="center">
  <h1>🏗️ Atlas Core Framework</h1>
  <p><em>Fundamental building blocks and abstractions for implementing Byzantine Fault Tolerant (BFT) and Crash Fault Tolerant (CFT) consensus protocols</em></p>

  [![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
</div>

---

## 📋 Table of Contents

- [Core Abstractions](#core-abstractions)
- [Key Components](#key-components)
- [Architecture Overview](#architecture-overview)
- [Using Atlas-Core](#using-atlas-core)
- [Implementation Guide](#implementation-guide)
- [Module Structure](#module-structure)

## 🏗️ Core Abstractions

### Ordering Protocol Framework

The primary abstraction in Atlas-Core is the `OrderingProtocol` trait, which defines the interface for implementing consensus algorithms:

```rust
pub trait OrderingProtocol<RQ>:
    OrderProtocolTolerance + Orderable + TimeoutableMod<OPExResult<RQ, Self::Serialization>>
{
    type Serialization: OrderingProtocolMessage<RQ> + 'static;
    type Config;

    fn handle_off_ctx_message(&mut self, message: ShareableConsensusMessage<RQ, Self::Serialization>);
    fn handle_execution_changed(&mut self, is_executing: bool) -> Result<()>;
    fn poll(&mut self) -> Result<OPResult<RQ, Self::Serialization>>;
    fn process_message(&mut self, message: ShareableConsensusMessage<RQ, Self::Serialization>) -> Result<OPExResult<RQ, Self::Serialization>>;
    fn install_seq_no(&mut self, seq_no: SeqNo) -> Result<()>;
}
```

### Permissioned Protocol Support

For protocols where only a set of nodes participate in consensus decisions:

```rust
pub trait PermissionedOrderingProtocol: OrderProtocolTolerance {
    type PermissionedSerialization: PermissionedOrderingProtocolMessage + 'static;
    
    fn view(&self) -> View<Self::PermissionedSerialization>;
    fn install_view(&mut self, view: View<Self::PermissionedSerialization>);
}
```

### Fault Tolerance Configuration

The `OrderProtocolTolerance` trait defines how protocols handle different fault scenarios:

```rust
pub trait OrderProtocolTolerance {
    fn get_n_for_f(f: usize) -> usize;
    fn get_quorum_for_n(n: usize) -> usize;
    fn get_f_for_n(n: usize) -> usize;
}
```

## 🔧 Key Components

### Decision Processing

Atlas-Core provides comprehensive decision handling through the `Decision` type, which tracks:

- **Decision Metadata**: Proof information and protocol state
- **Partial Decision Information**: Intermediate consensus progress
- **Decision Completion**: Final consensus results with executable batches

### Execution Framework

The `DecisionExecutorHandle` trait defines how consensus decisions are executed:

```rust
pub trait DecisionExecutorHandle<RQ>: Send + Clone + 'static {
    fn catch_up_to_quorum(&self, requests: MaybeVec<BatchedDecision<RQ>>) -> Result<()>;
    fn queue_update(&self, batch: BatchedDecision<RQ>) -> Result<()>;
    fn queue_update_unordered(&self, requests: Vec<StoredMessage<RQ>>) -> Result<()>;
}
```

### Persistent Storage

The `OrderingProtocolLog` trait provides durable storage for consensus state:

```rust
pub trait OrderingProtocolLog<RQ, OP>: Clone
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
{
    fn write_committed_seq_no(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()>;
    fn write_message(&self, write_mode: OperationMode, msg: ShareableConsensusMessage<RQ, OP>) -> Result<()>;
    fn write_decision_metadata(&self, write_mode: OperationMode, metadata: DecisionMetadata<RQ, OP>) -> Result<()>;
    // Additional persistence methods...
}
```

### Timeout Management

Sophisticated timeout handling through the `TimeoutableMod` trait and timeout identification system:

- Sequence number-based timeouts
- Session-based timeouts with node-specific tracking
- Configurable timeout policies and retransmission

### Request Pre-processing

The framework includes request batching and preprocessing capabilities:

```rust
pub trait WorkPartitioner: Send {
    fn get_worker_for<O>(rq_info: &Header, message: &O, worker_count: usize) -> usize
    where O: SessionBased;
    fn get_worker_for_processed(rq_info: &ClientRqInfo, worker_count: usize) -> usize;
}
```

## 🏛️ Architecture Overview

Atlas-Core follows a modular architecture with clear separation of concerns:

1. **Protocol Layer**: Core consensus algorithm implementations
2. **Networking Layer**: Message serialization, verification, and communication
3. **Storage Layer**: Persistent logging and checkpointing
4. **Execution Layer**: Request processing and state management
5. **Reconfiguration Layer**: Dynamic membership and parameter changes

### Message Flow

```
Client Requests → Pre-processing → Ordering Protocol → Decision → Execution → Response
                      ↓                    ↑               ↓
                 Work Division      Timeout Management   Persistent Log
```

## 🚀 Using Atlas-Core

### Basic Protocol Implementation

To implement a custom consensus protocol:

1. **Define your protocol structure**:
```rust
pub struct MyProtocol<RQ> {
    // Protocol-specific state
    view: MyView,
    sequence_number: SeqNo,
    // ... other fields
}
```

2. **Implement the core traits**:
```rust
impl<RQ> OrderingProtocol<RQ> for MyProtocol<RQ> {
    type Serialization = MyProtocolMessages;
    type Config = MyProtocolConfig;
    
    fn poll(&mut self) -> Result<OPResult<RQ, Self::Serialization>> {
        // Check for ready messages or required state transfers
    }
    
    fn process_message(&mut self, message: ShareableConsensusMessage<RQ, Self::Serialization>) 
        -> Result<OPExResult<RQ, Self::Serialization>> {
        // Handle incoming protocol messages
    }
    
    // Implement other required methods...
}
```

3. **Define message types**:
```rust
pub enum MyProtocolMessages {
    Prepare(PrepareMessage),
    Promise(PromiseMessage),
    Commit(CommitMessage),
}

impl<RQ> OrderingProtocolMessage<RQ> for MyProtocolMessages {
    type ProtocolMessage = Self;
    type DecisionMetadata = MyDecisionMetadata;
    type DecisionAdditionalInfo = MyAdditionalInfo;
    
    // Implement serialization and verification...
}
```

## 📚 Implementation Guide

### Fault Tolerance Models

Atlas-Core supports various fault tolerance models:

- **Byzantine Fault Tolerance**: For environments with potentially malicious nodes
- **Crash Fault Tolerance**: For fail-stop environments
- **Hybrid Models**: Combining different fault assumptions

### State Transfer Integration

The framework provides state transfer abstractions for handling node synchronization:

```rust
pub struct Checkpoint<S> {
    seq: SeqNo,
    app_state: S,
    digest: Digest,
}
```

### Reconfiguration Support

Dynamic system reconfiguration through:

- Membership changes
- Parameter updates
- View transitions
- Quorum modifications

## 📁 Module Structure

```
src/
├── ordering_protocol/          # Core consensus abstractions
│   ├── loggable/              # Persistent logging support
│   ├── networking/            # Message serialization & verification
│   ├── permissioned/          # Permissioned protocol extensions
│   └── reconfigurable_order_protocol/  # Dynamic reconfiguration
├── executor/                  # Execution framework
├── persistent_log/            # Durable storage abstractions
├── timeouts/                  # Timeout management system
├── request_pre_processing/    # Request batching & preprocessing
├── state_transfer/            # State synchronization protocols
├── reconfiguration_protocol/  # Dynamic membership management
├── messages/                  # Core message types
├── smr/                      # State Machine Replication support
└── metric/                   # Performance instrumentation
```

## 🔄 Protocol Execution Flow

1. **Initialization**: Protocol setup with configuration and network parameters
2. **Message Processing**: Handling incoming consensus messages
3. **Decision Making**: Reaching agreement on request ordering
4. **Persistence**: Logging decisions and protocol state
5. **Execution**: Processing decided requests through the executor
6. **State Management**: Handling checkpoints and state transfers

## ⚡ Advanced Features

- **Batch Processing**: Efficient request batching and preprocessing
- **Parallel Execution**: Support for concurrent request processing
- **Checkpointing**: Periodic state snapshots for recovery
- **Metrics Integration**: Comprehensive performance monitoring
- **Flexible Serialization**: Support for multiple serialization formats
- **Signature Verification**: Built-in cryptographic verification

## 🛠️ Development Guidelines

When implementing protocols with Atlas-Core:

1. **Follow the trait contracts**: Ensure all required methods are properly implemented
2. **Handle timeouts properly**: Implement timeout-based liveness mechanisms
3. **Manage state carefully**: Use proper synchronization for shared state
4. **Test fault scenarios**: Validate behavior under various failure conditions
5. **Monitor performance**: Utilize the built-in metrics system

Atlas-Core provides the foundation for building robust, efficient, and fault-tolerant consensus protocols within the Atlas framework ecosystem.
