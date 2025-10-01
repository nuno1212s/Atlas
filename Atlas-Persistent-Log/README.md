# Atlas-Persistent-Log

## Durable Storage Layer for the Atlas Framework

Atlas-Persistent-Log provides a robust and efficient persistent storage system for consensus decisions and protocol state within the Atlas framework. This module ensures that critical data is durably stored, enabling recovery from crashes and supporting system reconfiguration.

## Key Features

### Durable Storage

- **Log persistence**: Reliable storage of consensus decisions
- **Crash recovery**: Support for rebuilding state after system failures
- **Compaction**: Log compaction to manage storage growth
- **Checkpointing**: Integration with system checkpointing mechanisms

### Storage Abstractions

- **Decision log**: Storage for protocol decisions
- **State snapshots**: Periodic application state snapshots
- **Configuration state**: System configuration parameters
- **Recovery information**: Data needed for system recovery

## Core Components

### Persistent Log Interface

```rust
pub trait PersistentLog<D: Digest, S, CL>: Send + Sync {
    /// Store a consensus decision
    fn store_decision(&self, seq_no: SeqNo, decision: Decision<D>) -> Result<()>;
    
    /// Retrieve a consensus decision by sequence number
    fn retrieve_decision(&self, seq_no: SeqNo) -> Result<Option<Decision<D>>>;
    
    /// Store a checkpoint of the application state
    fn store_checkpoint(&self, seq_no: SeqNo, state: &S) -> Result<()>;
    
    /// Retrieve a checkpoint by sequence number
    fn retrieve_checkpoint(&self, seq_no: SeqNo) -> Result<Option<S>>;
    
    /// Store the current system configuration
    fn store_configuration(&self, config: CL) -> Result<()>;
    
    /// Retrieve the current system configuration
    fn retrieve_configuration(&self) -> Result<Option<CL>>;
    
    // Additional methods omitted for brevity
}
```

### Storage Backends

Atlas-Persistent-Log supports multiple storage backends through a pluggable architecture:

- **File-based**: Log files with append-only semantics
- **Key-value stores**: Using embedded KV stores like Sled
- **Custom implementations**: Support for custom storage backends

## Architecture

The module is structured as a layered system:

```
┌─────────────────────────────────────┐
│           Protocol Logic            │
└───────────────────┬─────────────────┘
                    │
┌───────────────────▼─────────────────┐
│         Persistent Log API          │
└───────────────────┬─────────────────┘
                    │
┌───────────────────▼─────────────────┐
│        Storage Abstraction          │
└───────────────────┬─────────────────┘
                    │
      ┌─────────────┴─────────────┐
      │                           │
┌─────▼─────┐             ┌───────▼─────┐
│ File-based │             │  Key-value  │
│  Storage   │             │   Storage   │
└───────────┘             └─────────────┘
```

## Usage

### Basic Usage

```rust
// Create a persistent log with the default backend
let log = FileBasedLog::new(config)?;

// Store a consensus decision
log.store_decision(seq_no, decision)?;

// Retrieve a decision
if let Some(decision) = log.retrieve_decision(seq_no)? {
    // Process the decision
}

// Store a checkpoint
log.store_checkpoint(seq_no, &application_state)?;

// Compact the log up to a sequence number
log.compact_log_up_to(checkpoint_seq_no)?;
```

### Recovery Process

```rust
// During system startup
let log = FileBasedLog::new(config)?;

// Retrieve the last stable checkpoint
let (last_checkpoint_seq, last_checkpoint) = log.retrieve_last_checkpoint()?;

// Retrieve all decisions after the last checkpoint
let decisions = log.retrieve_decisions_after(last_checkpoint_seq)?;

// Recover the system state
let current_state = recover_state(last_checkpoint, decisions)?;
```

## Configuration

Atlas-Persistent-Log can be configured with various parameters:

```rust
pub struct PersistentLogConfig {
    /// Directory for log storage
    pub log_dir: PathBuf,
    
    /// Maximum log segment size
    pub max_segment_size: usize,
    
    /// Sync policy (e.g., every write, periodic)
    pub sync_policy: SyncPolicy,
    
    /// Compression options
    pub compression: CompressionOptions,
    
    /// Cache size for recently accessed entries
    pub cache_size: usize,
    
    // Additional configuration options
}
```

## Performance Optimizations

- **Batched writes**: Group multiple writes for improved throughput
- **Asynchronous I/O**: Background flushing to reduce latency
- **Caching**: In-memory caching of frequently accessed data
- **Compression**: Optional compression to reduce storage requirements

## Integration with Atlas

Atlas-Persistent-Log integrates with other Atlas components:

- **Atlas-Core**: Provides persistence for core protocol state
- **Atlas-SMR-Replica**: Supports replica recovery and checkpointing
- **Atlas-Reconfiguration**: Stores configuration changes durably

## Recovery Mechanisms

- **Crash recovery**: Rebuild state after system crashes
- **Partial recovery**: Recover only necessary parts of the log
- **Incremental recovery**: Apply log entries incrementally
- **Checkpoint-based recovery**: Recover from the latest checkpoint

## Performance Considerations

- **Write amplification**: Techniques to minimize redundant writes
- **Read optimization**: Indexing for efficient read access
- **Compaction strategies**: Policies for log compaction
- **Durability vs. performance**: Configurable trade-offs

## License

This module is licensed under the MIT License - see the LICENSE.txt file for details.
