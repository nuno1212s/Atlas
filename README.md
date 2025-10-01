# Atlas-SMR-Core

<div align="center">
  <h1>🏗️ Atlas SMR Core Framework</h1>
  <p><em>State Machine Replication abstractions and components built on top of Atlas-Core for implementing fault-tolerant distributed applications</em></p>

  [![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
</div>

---

## 📋 Table of Contents

- [Core Abstractions](#core-abstractions)
- [Key Components](#key-components)
- [Architecture Overview](#architecture-overview)
- [Integration with Atlas-Core](#integration-with-atlas-core)
- [Implementation Guide](#implementation-guide)
- [Module Structure](#module-structure)

## 🏗️ Core Abstractions

Atlas-SMR-Core provides high-level abstractions for State Machine Replication that build upon the consensus protocols defined in Atlas-Core. The framework enables the development of fault-tolerant distributed applications by providing message handling, execution coordination, and state management components.

### State Machine Replication Types

The framework defines fundamental SMR message types that integrate with Atlas-Core's request-response abstraction:

```rust
pub type SMRReq<D: ApplicationData> = RequestMessage<D::Request>;
pub type SMRRawReq<R> = RequestMessage<R>;
pub type SMRReply<D: ApplicationData> = ReplyMessage<D::Reply>;
```

### Execution Framework

#### State Executor Integration

The `WrappedExecHandle` provides a bridge between Atlas-Core's `DecisionExecutorHandle` trait and SMR-specific application execution:

```rust
pub struct WrappedExecHandle<R>(pub ExecutorHandle<R>);

impl<R> DecisionExecutorHandle<SMRRawReq<R>> for WrappedExecHandle<R> {
    fn catch_up_to_quorum(&self, requests: MaybeVec<BatchedDecision<SMRRawReq<R>>>) -> Result<()>;
    fn queue_update(&self, batch: BatchedDecision<SMRRawReq<R>>) -> Result<()>;
    fn queue_update_unordered(&self, requests: Vec<StoredMessage<SMRRawReq<R>>>) -> Result<()>;
}
```

#### Reply Node Abstraction

The `ReplyNode` trait defines how replicas send responses back to clients:

```rust
pub trait ReplyNode<RP>: Send + Sync {
    fn send(&self, reply_type: RequestType, reply: RP, target: NodeId, flush: bool) -> Result<()>;
    fn send_signed(&self, reply_type: RequestType, reply: RP, target: NodeId, flush: bool) -> Result<()>;
    fn broadcast(&self, reply_type: RequestType, reply: RP, targets: impl Iterator<Item = NodeId>) -> std::result::Result<(), Vec<NodeId>>;
    fn broadcast_signed(&self, reply_type: RequestType, reply: RP, targets: impl Iterator<Item = NodeId>) -> std::result::Result<(), Vec<NodeId>>;
}
```

### Message System Architecture

#### Orderable Message Framework

The `OrderableMessage` enum handles both ordered and unordered requests and replies:

```rust
pub enum OrderableMessage<D: ApplicationData> {
    OrderedRequest(SMRReq<D>),
    UnorderedRequest(SMRReq<D>),
    OrderedReply(SMRReply<D>),
    UnorderedReply(SMRReply<D>),
}
```

#### System Message Integration

The `SystemMessage` enum encapsulates all inter-replica communication, integrating Atlas-Core's protocol abstractions:

```rust
pub enum SystemMessage<D: ApplicationData, P, LT, VT> {
    ForwardedRequestMessage(ForwardedRequestsMessage<SMRReq<D>>),
    ProtocolMessage(Protocol<P>),
    ForwardedProtocolMessage(ForwardedProtocolMessage<P>),
    LogTransferMessage(LogTransfer<LT>),
    ViewTransferMessage(VTMessage<VT>),
}
```

## 🔧 Key Components

### Networking Layer

#### SMR Replica Network Node

The `SMRReplicaNetworkNode` trait defines a comprehensive networking abstraction for SMR replicas:

```rust
pub trait SMRReplicaNetworkNode<NI, RM, D, P, L, VT, S> {
    type ProtocolNode: OrderProtocolSendNode<SMRReq<D>, P>
        + LogTransferSendNode<SMRReq<D>, P, L>
        + ViewTransferProtocolSendNode<VT>
        + RegularNetworkStub<Service<D, P, L, VT>>;

    type ApplicationNode: RequestPreProcessingHandle<SMRSysMessage<D>> + ReplyNode<SMRReply<D>>;
    type StateTransferNode: StateTransferSendNode<S> + RegularNetworkStub<StateSys<S>>;
    type ReconfigurationNode: RegularNetworkStub<RM>;

    fn protocol_node(&self) -> &Arc<Self::ProtocolNode>;
    fn app_node(&self) -> &Arc<Self::ApplicationNode>;
    fn state_transfer_node(&self) -> &Arc<Self::StateTransferNode>;
    fn reconfiguration_node(&self) -> &Arc<Self::ReconfigurationNode>;
}
```

### Request Pre-Processing Framework

#### Pre-Processing Orchestration

The `RequestPreProcessor` implements Atlas-Core's `RequestPreProcessing` trait, providing batch processing and request forwarding capabilities:

```rust
impl<O> RequestPreProcessing<O> for RequestPreProcessor<O> {
    fn process_forwarded_requests(&self, message: StoredMessage<ForwardedRequestsMessage<O>>) -> Result<()>;
    fn process_stopped_requests(&self, messages: Vec<StoredMessage<O>>) -> Result<()>;
    fn process_decided_batch(&self, client_rqs: Vec<ClientRqInfo>) -> Result<()>;
}
```

#### Worker-Based Processing

The framework includes a worker-based architecture for parallel request processing, with dedicated worker threads handling request batching and validation.

### State Transfer Protocol

#### State Transfer Abstraction

The `StateTransferProtocol` trait provides comprehensive state synchronization capabilities:

```rust
pub trait StateTransferProtocol<S>: TimeoutableMod<STTimeoutResult> {
    type Serialization: StateTransferMessage + 'static;

    fn request_latest_state<V>(&mut self, view: V) -> Result<()> where V: NetworkView;
    fn poll(&mut self) -> Result<STPollResult<CstM<Self::Serialization>>>;
    fn process_message<V>(&mut self, view: V, message: StoredMessage<CstM<Self::Serialization>>) -> Result<STResult> where V: NetworkView;
    fn handle_app_state_requested(&mut self, seq: SeqNo) -> Result<ExecutionResult>;
}
```

#### Checkpoint Management

The `Checkpoint` structure provides state snapshot capabilities:

```rust
#[derive(Clone)]
pub struct Checkpoint<S> {
    seq: SeqNo,
    app_state: S,
    digest: Digest,
}

impl<S> Checkpoint<S> {
    pub fn new(seq: SeqNo, app_state: S, digest: Digest) -> Arc<ReadOnly<Self>>;
    pub fn state(&self) -> &S;
    pub fn digest(&self) -> &Digest;
}
```

### Persistent Storage Abstractions

#### Monolithic State Logging

For applications with atomic state snapshots:

```rust
pub trait MonolithicStateLog<S>: Send where S: MonolithicState {
    fn read_checkpoint(&self) -> Result<Option<Checkpoint<S>>>;
    fn write_checkpoint(&self, write_mode: OperationMode, checkpoint: Arc<ReadOnly<Checkpoint<S>>>) -> Result<()>;
}
```

#### Divisible State Logging

For applications with partitionable state:

```rust
pub trait DivisibleStateLog<S>: Send where S: DivisibleState {
    fn read_local_descriptor(&self) -> Result<Option<S::StateDescriptor>>;
    fn write_parts(&self, write_mode: OperationMode, parts: Vec<Arc<ReadOnly<S::StatePart>>>) -> Result<()>;
    fn write_parts_and_descriptor(&self, write_mode: OperationMode, descriptor: S::StateDescriptor, parts: Vec<Arc<ReadOnly<S::StatePart>>>) -> Result<()>;
}
```

## 🏛️ Architecture Overview

Atlas-SMR-Core creates a layered architecture that extends Atlas-Core's consensus foundations:

```
Application Layer     ← SMR Application Logic
    ↕
SMR-Core Layer       ← Message Routing, State Transfer, Request Processing
    ↕
Atlas-Core Layer     ← Consensus Protocols, Timeouts, Decision Processing
    ↕
Communication Layer  ← Network I/O, Serialization, Cryptography
```

### Message Flow Architecture

```
Client Requests → Application Node → Request Pre-Processing → Protocol Node → Consensus → Execution
                      ↕                       ↕                    ↕           ↕
                 Reply Handling        Worker Threads      State Transfer  Persistent Log
```

## 🔗 Integration with Atlas-Core

Atlas-SMR-Core implements and extends several Atlas-Core abstractions:

### Execution Integration

- **`DecisionExecutorHandle`**: `WrappedExecHandle` transforms Atlas-Core's `BatchedDecision` into SMR-specific `UpdateBatch` and `UnorderedBatch` types
- **Request Processing**: Integrates with Atlas-Core's `RequestPreProcessing` and `WorkPartitioner` traits

### Protocol Integration

- **Message Serialization**: `Service` type implements Atlas-Core's `Serializable` trait for protocol messages
- **Network Integration**: SMR networking nodes implement Atlas-Core's various `SendNode` traits
- **Timeout Management**: State transfer protocols implement Atlas-Core's `TimeoutableMod` trait

### Persistent Log Integration

- **Decision Logging**: Extends Atlas-Core's `OrderingProtocolLog` with SMR-specific state checkpointing
- **State Management**: Provides additional persistence abstractions for application state

## 🚀 Implementation Guide

### Basic SMR Application Setup

1. **Define Application Data Types**:
```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct MyRequest { /* ... */ }

#[derive(Clone, Serialize, Deserialize)]  
pub struct MyReply { /* ... */ }

pub struct MyApp;

impl ApplicationData for MyApp {
    type Request = MyRequest;
    type Reply = MyReply;
}
```

2. **Implement Networking**:
```rust
pub struct MyReplicaNode<CN, BN, NI, RM, P, L, VT, S> {
    // Use ReplicaNodeWrapper or implement SMRReplicaNetworkNode directly
}
```

3. **Configure State Transfer**:
```rust
pub struct MyStateTransfer;

impl<S> StateTransferProtocol<S> for MyStateTransfer {
    type Serialization = MyStateTransferMessages;
    
    // Implement required methods...
}
```

### Request Processing Pipeline

1. **Client Request Reception**: Application node receives requests
2. **Pre-Processing**: Requests are batched and validated by worker threads
3. **Protocol Processing**: Consensus protocol orders requests using Atlas-Core
4. **Execution**: `WrappedExecHandle` transforms decisions for application execution
5. **Reply Generation**: Results are sent back through the reply node

## 📁 Module Structure

```
src/
├── exec/                    # Execution framework and reply handling
├── message/                 # SMR message types and system messages
├── networking/             # Network abstraction and client handling
│   ├── client.rs          # Client-specific networking
│   └── signature_ver/     # Message verification
├── persistent_log/         # State persistence abstractions
├── request_pre_processing/ # Request batching and preprocessing
│   └── worker/            # Worker thread implementation
├── serialize/              # Message serialization framework
└── state_transfer/         # State synchronization protocols
    ├── divisible_state/   # Partitionable state support
    ├── monolithic_state/  # Atomic state support
    └── networking/        # State transfer networking
```

## ⚡ Advanced Features

### Request Type Support
- **Ordered Requests**: Go through consensus protocol for total ordering
- **Unordered Requests**: Bypass consensus for read-only operations
- **Request Forwarding**: Automatic request routing between replicas

### State Transfer Modes
- **Monolithic State Transfer**: For applications with atomic state
- **Divisible State Transfer**: For applications with partitionable state
- **Incremental Synchronization**: Efficient state updates between checkpoints

### Performance Optimizations
- **Worker Thread Pools**: Parallel request processing
- **Batch Processing**: Efficient request aggregation
- **Signature Verification**: Optimized cryptographic operations
- **Connection Management**: Efficient network resource utilization

## 🛠️ Development Guidelines

When building SMR applications with Atlas-SMR-Core:

1. **Leverage Existing Abstractions**: Use the provided networking and execution frameworks
2. **Implement Required Traits**: Ensure proper integration with Atlas-Core protocols
3. **Handle Both Request Types**: Support both ordered and unordered operations appropriately
4. **Design for State Transfer**: Consider checkpoint frequency and state partitioning
5. **Monitor Performance**: Utilize the integrated metrics system for optimization

Atlas-SMR-Core provides a complete framework for building robust, high-performance State Machine Replication applications on top of the Atlas consensus infrastructure.
