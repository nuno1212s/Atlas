# Atlas-SMR-Replica

<div align="center">
  <h1>🏗️ Atlas SMR Replica Orchestrator</h1>
  <p><em>The central orchestrator that coordinates all Atlas framework components to implement a complete Byzantine Fault Tolerant State Machine Replication replica</em></p>

  [![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
</div>

---

## 📋 Table of Contents

- [Overview](#-overview)
- [Core Architecture](#-core-architecture)
- [Orchestration Framework](#-orchestration-framework)
- [Component Integration](#-component-integration)
- [Execution Models](#-execution-models)
- [Communication Architecture](#-communication-architecture)
- [Lifecycle Management](#-lifecycle-management)
- [Configuration](#-configuration)
- [Usage Examples](#-usage-examples)
- [Performance Considerations](#-performance-considerations)

## 🏗️ Overview

Atlas-SMR-Replica serves as the **central orchestrator** for the entire Atlas State Machine Replication system. Unlike a simple replica implementation, it coordinates and manages the complex interactions between all Atlas framework components to provide a complete BFT SMR solution.

## 🏛️ Core Architecture

Atlas-SMR-Replica implements a sophisticated multi-layered orchestration architecture:

```
┌─────────────────────────────────────────────────────────────────┐
│                        Client Layer                             │
└───────────────────────┬─────────────────────────────────────────┘
                        │
         ┌──────────────▼──────────────┐
         │   Atlas-SMR-Replica         │
         │    (Orchestrator)           │
         └──┬────────────────────────┬──┘
            │                        │
┌───────────▼─────────────┐ ┌───────▼──────────────┐
│    Protocol Layer       │ │   Execution Layer    │
│ ┌─────────────────────┐ │ │ ┌──────────────────┐ │
│ │ Ordering Protocol   │ │ │ │ Atlas-SMR-       │ │
│ │ (Consensus)         │ │ │ │ Execution        │ │
│ └─────────────────────┘ │ │ └──────────────────┘ │
│ ┌─────────────────────┐ │ │ ┌──────────────────┐ │
│ │ View Transfer       │ │ │ │ Atlas-SMR-       │ │
│ │ Protocol            │ │ │ │ Application      │ │
│ └─────────────────────┘ │ │ └──────────────────┘ │
│ ┌─────────────────────┐ │ └──────────────────────┘
│ │ Reconfiguration     │ │
│ │ Protocol            │ │
│ └─────────────────────┘ │
└─────────────────────────┘
            │
┌───────────▼─────────────┐
│    Storage Layer        │
│ ┌─────────────────────┐ │
│ │ Decision Log        │ │
│ │ Management          │ │
│ └────────���────────────┘ │
│ ┌─────────────────────┐ │
│ │ State Transfer      │ │
│ │ Protocols           │ │
│ └─────────────────────┘ │
│ ┌─────────────────────┐ │
│ │ Log Transfer        │ │
│ │ Protocols           │ │
│ └─────────────────────┘ │
│ ┌─────────────────────┐ │
│ │ Persistent Log      │ │
│ └─────────────────────┘ │
└─────────────────────────┘
            │
┌───────────▼─���───────────┐
│  Communication Layer    │
│ ┌───���─────────────────┐ │
│ │ Atlas-Communication │ │
│ │ Network I/O         │ │
│ └─────────────────────┘ │
└─────────────────────────┘
```

## ⚙️ Orchestration Framework

### Central Replica Coordinator

The `Replica<RP, S, D, OP, DL, ST, LT, VT, NT, PL>` struct serves as the central orchestrator with multiple execution phases:

```rust
pub struct Replica<...> {
    execution_state: ExecutionPhase,      // Current protocol phase
    transfer_states: TransferPhase,       // State/Log transfer coordination
    ordering_protocol: OP,                // Consensus protocol
    view_transfer_protocol: VT,           // View change coordination
    decision_log_handle: DecisionLogHandle, // Decision logging coordination
    executor_handle: WrappedExecHandle,   // Application execution bridge
    state_transfer_handle: StateTransferThreadHandle, // State sync coordination
    // ... additional orchestration components
}
```

### Execution Phase Management

The orchestrator manages multiple execution phases:

```rust
pub enum ExecutionPhase {
    OrderProtocol,        // Normal consensus operation
    ViewTransferProtocol, // Leader change/view synchronization
}

pub enum TransferPhase {
    NotRunning,
    RunningTransferProtocols {
        state_transfer: StateTransferState,  // Application state sync
        log_transfer: LogTransferState,      // Decision log sync
    },
}
```

### State Orchestration Models

Atlas-SMR-Replica supports two distinct orchestration models:

#### Monolithic State Orchestration
```rust
pub struct MonReplica<RP, ME, S, A, OP, DL, ST, LT, VT, NT, PL>
```
- Coordinates atomic state management
- Orchestrates single-threaded or multi-threaded monolithic execution
- Manages complete state checkpointing and transfer

#### Divisible State Orchestration  
```rust
pub struct DivStReplica<RP, SE, S, A, OP, DL, ST, LT, VT, NT, PL>
```
- Coordinates partitioned state management
- Orchestrates incremental state transfer protocols
- Manages state part synchronization and consistency

## 🔗 Component Integration

### Protocol Integration Matrix

The orchestrator manages complex interactions between Atlas components:

| Component | Integration Point | Orchestration Role |
|-----------|------------------|-------------------|
| **Atlas-Core** | `OrderingProtocol` | Coordinates consensus decisions and view changes |
| **Atlas-SMR-Core** | `SMRReplicaNetworkNode` | Manages SMR-specific networking and message routing |
| **Atlas-SMR-Application** | `WrappedExecHandle` | Bridges consensus decisions to application execution |
| **Atlas-SMR-Execution** | Executor Integration | Coordinates execution model selection and management |
| **Atlas-Communication** | `NetworkNode` | Manages all network I/O and message serialization |
| **Atlas-Persistent-Log** | `SMRPersistentLog` | Coordinates persistent storage and recovery |
| **Atlas-Logging-Core** | `DecisionLog` + `LogTransferProtocol` | Manages decision persistence and log synchronization |
| **Atlas-Metrics** | Metrics Integration | Coordinates performance monitoring across all components |

### Message Flow Orchestration

```
Client Request → Request Pre-Processing → Protocol Consensus → Decision Logging
                      ↕                         ↕                    ↕
                 Worker Threads          State Management      Persistent Storage
                      ↕                         ↕                    ↕
                 Request Batching        Execution Engine     State Transfer
                      ↕                         ↕                    ↕
                Network Routing          Application Logic     Log Transfer
                      ↕                         ↕                    ↕
                  Reply Handling           State Updates        Recovery
```

### Thread Coordination Architecture

The orchestrator manages multiple concurrent threads:

```rust
// Main orchestrator thread - coordinates all components
fn iterate(&mut self) -> Result<()> {
    // Coordinate protocol execution phases
    // Manage component interactions
    // Handle timeout coordination
    // Process network messages
}

// Decision log thread - manages persistent logging
DecisionLogManager::initialize_decision_log_mngt()

// State transfer thread - manages state synchronization  
StateTransferMngr::initialize_core_state_transfer()

// Execution threads - managed by Atlas-SMR-Execution
// Request pre-processing threads - managed by Atlas-SMR-Core
// Network I/O threads - managed by Atlas-Communication
```

## 🚀 Execution Models

### Orchestration by State Type

#### Monolithic State Orchestration

For applications with atomic, indivisible state:

```rust
impl<...> MonReplica<...> {
    pub async fn bootstrap(cfg: MonolithicStateReplicaConfig<...>) -> Result<Self>
    pub async fn run(self) -> Result<()>
}
```

**Orchestration Features:**
- Coordinates complete state checkpointing
- Manages atomic state transfer during recovery
- Supports both single-threaded and multi-threaded execution models
- Orchestrates state validation and integrity checks

#### Divisible State Orchestration

For applications with partitionable state:

```rust  
impl<...> DivStReplica<...> {
    pub async fn bootstrap(cfg: DivisibleStateReplicaConfig<...>) -> Result<Self>
    pub async fn run(self) -> Result<()>
}
```

**Orchestration Features:**
- Coordinates incremental state transfer
- Manages state part synchronization
- Supports parallel state part processing
- Orchestrates efficient state delta management

### Execution Model Coordination

The orchestrator integrates with Atlas-SMR-Execution to coordinate:

- **Single-threaded execution:** Sequential request processing
- **Multi-threaded execution:** Speculative parallel execution with conflict detection
- **Mixed execution:** Ordered and unordered request handling
- **Checkpoint coordination:** Periodic state snapshot generation

## 📡 Communication Architecture

### Network Node Orchestration

```rust
pub trait SMRReplicaNetworkNode<NI, RM, D, P, L, VT, S> {
    type ProtocolNode: // Consensus and protocol message handling
    type ApplicationNode: // Client request/reply handling  
    type StateTransferNode: // State synchronization messaging
    type ReconfigurationNode: // Membership change messaging
}
```

### Message Type Coordination

The orchestrator handles multiple message types:

```rust
pub enum SystemMessage<D, P, LT, VT> {
    ForwardedRequestMessage(ForwardedRequestsMessage<SMRReq<D>>), // Client requests
    ProtocolMessage(Protocol<P>),                                 // Consensus messages
    ForwardedProtocolMessage(ForwardedProtocolMessage<P>),       // Forwarded protocol messages
    LogTransferMessage(LogTransfer<LT>),                         // Log synchronization
    ViewTransferMessage(VTMessage<VT>),                          // View change messages
}
```

### Channel-Based Coordination

The orchestrator uses multiple communication channels:

```rust
// Network message reception
recv(network_node.protocol_node().incoming_stub()) -> network_msg

// Decision log coordination  
recv(decision_log_handle.status_rx()) -> status_msg

// State transfer coordination
recv(state_transfer_handle.response_rx()) -> state_msg

// Network updates
recv(network_update_listener) -> network_update

// Reconfiguration coordination
recv(reconf_receive) -> reconf_msg

// Timeout coordination across all components
recv(timeout_rx) -> timeout
```

## ⏱️ Lifecycle Management

### Bootstrap Orchestration

```rust
async fn bootstrap(cfg: ReplicaConfig<...>) -> Result<Self> {
    // 1. Initialize networking and reconfiguration
    let network = initialize_network_protocol().await?;
    let reconfig_protocol = initialize_reconfiguration_protocol().await?;
    
    // 2. Establish quorum consensus
    let quorum = acquire_quorum_info()?;
    
    // 3. Initialize request processing pipeline
    let (ordered, unordered) = initialize_rq_pre_processor()?;
    
    // 4. Initialize persistent storage
    let persistent_log = initialize_persistent_log()?;
    
    // 5. Initialize consensus protocols
    let ordering_protocol = initialize_order_protocol()?;
    let view_transfer_protocol = initialize_view_transfer_protocol()?;
    
    // 6. Initialize decision and state management
    let decision_handle = initialize_decision_log()?;
    let state_handle = initialize_state_transfer()?;
    
    // 7. Coordinate all components into orchestrator
    Self::coordinate_components(...)
}
```

### Runtime Orchestration Loop

```rust
pub async fn run(mut self) -> Result<()> {
    self.bootstrap_protocols()?;
    
    loop {
        // Coordinate all protocol execution
        self.iterate()?;
        
        // Handle graceful shutdown
        if shutdown_requested { break; }
    }
}

fn iterate(&mut self) -> Result<()> {
    // Phase-based execution coordination
    match self.execution_state {
        ExecutionPhase::OrderProtocol => self.run_order_protocol()?,
        ExecutionPhase::ViewTransferProtocol => self.iterate_view_transfer_protocol()?,
    };
    
    // Coordinate inter-component communication
    self.receive_internal_select()?;
}
```

### Recovery and State Synchronization

The orchestrator manages complex recovery scenarios:

1. **State Transfer Coordination:** Synchronizes application state with other replicas
2. **Log Transfer Coordination:** Ensures decision log consistency  
3. **Execution Catch-up:** Replays missed decisions after recovery
4. **Protocol State Recovery:** Restores consensus protocol state

## ⚙️ Configuration

### Comprehensive Configuration Structure

```rust
pub struct ReplicaConfig<RF, S, D, OP, DL, ST, LT, VT, NT, PL> {
    pub next_consensus_seq: SeqNo,                    // Recovery sequence number
    pub db_path: String,                              // Persistent storage location
    pub op_config: OP::Config,                        // Consensus protocol configuration
    pub dl_config: DL::Config,                        // Decision log configuration  
    pub lt_config: LT::Config,                        // Log transfer configuration
    pub pl_config: PL::Config,                        // Persistent log configuration
    pub vt_config: VT::Config,                        // View transfer configuration
    pub node: NT::Config,                             // Network configuration
    pub reconfig_node: RF::Config,                    // Reconfiguration configuration
    pub preprocessor_threads: usize,                  // Request processing threads
}
```

### Specialized Configurations

#### Monolithic State Configuration
```rust
pub struct MonolithicStateReplicaConfig<RF, S, A, OP, DL, ST, LT, VT, NT, PL> {
    pub service: A,                                   // Application logic
    pub replica_config: ReplicaConfig<...>,          // Base replica configuration
    pub st_config: ST::Config,                        // State transfer configuration
}
```

#### Divisible State Configuration  
```rust
pub struct DivisibleStateReplicaConfig<RF, S, A, OP, DL, ST, LT, VT, NT, PL> {
    pub service: A,                                   // Application logic
    pub replica_config: ReplicaConfig<...>,          // Base replica configuration  
    pub st_config: ST::Config,                        // State transfer configuration
}
```

## 🚀 Usage Examples

### Basic Monolithic State Replica

```rust
use atlas_smr_replica::server::monolithic_server::MonReplica;
use atlas_smr_replica::config::MonolithicStateReplicaConfig;

// Define your application
pub struct MyApp;
impl Application<MyState> for MyApp { /* ... */ }

// Define your state
#[derive(Clone)]
pub struct MyState { /* ... */ }
impl MonolithicState for MyState { /* ... */ }

// Configure and run replica
#[tokio::main]
async fn main() -> Result<()> {
    let config = MonolithicStateReplicaConfig {
        service: MyApp,
        replica_config: ReplicaConfig {
            op_config: consensus_config,
            node: network_config,
            preprocessor_threads: 4,
            // ... other configurations
        },
        st_config: state_transfer_config,
    };
    
    let replica = MonReplica::<
        MyReconfigProtocol, MyExecutor, MyState, MyApp,
        MyOrderingProtocol, MyDecisionLog, MyStateTransfer,
        MyLogTransfer, MyViewTransfer, MyNetworkNode, MyPersistentLog
    >::bootstrap(config).await?;
    
    replica.run().await
}
```

### Advanced Divisible State Replica

```rust
use atlas_smr_replica::server::divisible_state_server::DivStReplica;
use atlas_smr_replica::config::DivisibleStateReplicaConfig;

// Application with partitionable state
impl DivisibleState for MyPartitionedState {
    type PartDescription = MyPartId;
    type StateDescriptor = MyStateDescriptor;
    type StatePart = MyStatePart;
    // ... implementation
}

// Multi-threaded scalable application
impl ScalableApp<MyPartitionedState> for MyScalableApp {
    fn speculatively_execute(&self, state: &mut impl CRUDState, request: MyRequest) -> MyReply {
        // Speculative execution logic with conflict detection
    }
}

let config = DivisibleStateReplicaConfig {
    service: MyScalableApp,
    replica_config: ReplicaConfig {
        preprocessor_threads: 8,  // More threads for higher throughput
        // ... configuration for partitioned state handling
    },
    st_config: divisible_state_config,
};

let replica = DivStReplica::<
    MyReconfigProtocol, MyMultiThreadedExecutor, MyPartitionedState,
    MyScalableApp, MyOrderingProtocol, MyDecisionLog, MyDivisibleStateTransfer,
    MyLogTransfer, MyViewTransfer, MyNetworkNode, MyPersistentLog
>::bootstrap(config).await?;
```

## 📊 Performance Considerations

### Orchestration Optimizations

- **Phase-based Execution:** Minimizes coordination overhead through structured execution phases
- **Channel-based Communication:** High-performance bounded channels for inter-component communication
- **Selective Message Processing:** Biased message processing prioritizes critical protocol messages
- **Lazy State Transfer:** State synchronization only when necessary
- **Parallel Component Coordination:** Multiple components operate concurrently under orchestration

### Tuning Parameters

```rust
// Channel buffer sizes for component coordination
const REPLICA_MESSAGE_CHANNEL: usize = 1024;      // Main coordination channel
const WORK_CHANNEL_SIZE: usize = 128;             // State transfer work channel  
const RESPONSE_CHANNEL_SIZE: usize = 128;         // Response coordination channel

// Checkpoint coordination
pub const CHECKPOINT_PERIOD: u32 = 1000;          // Decisions between checkpoints

// Timeout coordination
pub const REPLICA_WAIT_TIME: Duration = Duration::from_millis(1000);
```

### Monitoring and Metrics

The orchestrator provides comprehensive metrics:

- **Protocol Coordination:** `ORDERING_PROTOCOL_POLL_TIME_ID`, `TIMEOUT_PROCESS_TIME_ID`
- **Component Integration:** `OP_MESSAGES_PROCESSED_ID`, `REPLICA_PROTOCOL_RESP_PROCESS_TIME_ID`
- **Execution Coordination:** `REPLICA_ORDERED_RQS_PROCESSED_ID`, `DECISION_LOG_PROCESSED_ID`
- **Performance Tracking:** `RUN_LATENCY_TIME_ID`, `EXECUTION_TIME_TAKEN_ID`

## 🔧 Advanced Features

### Dynamic Reconfiguration Orchestration

```rust
pub trait ReconfigurableProtocolHandling {
    fn attempt_quorum_join(&mut self, node: NodeId) -> Result<()>;
    fn attempt_to_join_quorum(&mut self) -> Result<()>;
}
```

The orchestrator coordinates membership changes:
- Manages node join/leave operations across all components
- Coordinates protocol state updates during reconfiguration
- Ensures consistency during membership transitions

### Fault Tolerance Coordination

- **Byzantine Fault Detection:** Coordinates detection across networking and consensus layers
- **Recovery Orchestration:** Manages state and log synchronization during recovery
- **Graceful Degradation:** Continues operation with reduced functionality during partial failures

### State Consistency Guarantees

- **Multi-Protocol Synchronization:** Ensures state/log transfer protocols remain synchronized
- **Checkpoint Coordination:** Manages consistent checkpointing across all storage layers
- **Recovery Validation:** Validates state consistency after recovery operations

## 📄 License

This module is licensed under the MIT License - see the LICENSE.txt file for details.

---

Atlas-SMR-Replica serves as the sophisticated orchestration layer that transforms the individual Atlas framework components into a cohesive, high-performance Byzantine Fault Tolerant State Machine Replication system. Rather than being just another replica implementation, it provides the essential coordination and integration logic that makes the entire Atlas ecosystem work together seamlessly.
