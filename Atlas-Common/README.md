# Atlas-Common

<div align="center">

**🔧 Core Utilities and Abstractions for the Atlas BFT Framework**

*A highly modular, feature-rich foundation library for distributed systems*

[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

</div>

---

## 📖 Overview

Atlas-Common serves as the foundational layer for the Atlas Byzantine Fault Tolerant (BFT) framework. It provides a comprehensive collection of utilities, abstractions, and pluggable components that enable building robust distributed systems with customizable backends.

**Key Philosophy**: *Software variation is encouraged* - employ different backend libraries performing identical duties to enhance system resilience in BFT environments.

## ✨ Features

### 🚀 Async Runtime Support
Choose your preferred async runtime:
- **🔥 Tokio** (default) - High-performance async runtime
- **🌊 async-std** - Alternative async runtime

### 🧵 Thread Pool Management
CPU-intensive task execution:
- **⚡ Rayon** (default) - Work-stealing thread pool
- **🔀 Crossbeam** - Alternative thread pool implementation

### 🌐 Network Communication
Flexible socket implementations:
- **🔌 Tokio TCP** (default) - Async TCP with Tokio
- **🌐 async-std TCP** - TCP with async-std
- **⚡ Rio TCP** - High-performance TCP with Rio

### 📡 Channel Systems
Various channel implementations for inter-component communication:
- **📬 Flume MPMC** (default) - Multi-producer, multi-consumer channels
- **🔄 Crossbeam Sync** (default) - Synchronous channels
- **🎭 Mixed Flume** (default) - Hybrid async/sync channels
- **📦 Custom Dump** (default) - Specialized multiplexed channels
- **⚡ Async Channel MPMC** - Alternative async channels

### 🔐 Cryptographic Operations
Secure operations with pluggable crypto backends:

#### Digital Signatures
- **✍️ Ring Ed25519** (default) - EdDSA signatures using Ring

#### Hashing
- **🔨 Blake3** (default) - Fast cryptographic hash function
- **🔗 Ring SHA2** - SHA-2 family hash functions

#### Threshold Cryptography
- **🤝 Threshold Crypto** - Distributed key generation and threshold signatures
- **❄️ FROST Ed25519** - Flexible Round-Optimized Schnorr Threshold signatures

### 📊 Collections & Data Structures
Optimized data structures:
- **🦀 FxHash** (default) - Fast hash functions for collections
- **🔢 TwoX Hash** - Alternative hash implementation
- **📋 Specialized Collections** - Distributed systems optimized structures

### 💾 Persistent Storage
Pluggable database backends:
- **🌲 Sled** (default) - Embedded database
- **🪨 RocksDB** - High-performance key-value store
- **🚫 Disabled** - No persistence (testing/development)

### 🔄 Serialization
- **📦 Serde** (default) - Flexible serialization framework

### 🛡️ Additional Components
- **⚡ Circuit Breaker** - Prevent cascading failures
- **🎯 Node ID Management** - Distributed node identification
- **🌍 Global State Management** - Shared configuration and state
- **🎲 PRNG** - Pseudorandom number generation
- **📐 System Parameters** - Runtime configuration management
- **🔍 Error Handling** - Comprehensive error propagation

## 🚀 Quick Start

### Default Configuration
```toml
[dependencies]
atlas-common = { path = "../Atlas-Common" }
```

### Custom Configuration
```toml
[dependencies]
atlas-common = { 
    path = "../Atlas-Common", 
    default-features = false, 
    features = [
        "async_runtime_tokio",
        "threadpool_rayon",
        "socket_tokio_tcp",
        "crypto_hash_ring_sha2",
        "serialize_serde"
    ]
}
```

### Initialization
```rust
use atlas_common::{init, InitConfig};

// Initialize the library with custom thread counts
let _guard = unsafe {
    init(InitConfig {
        async_threads: 4,
        threadpool_threads: 8,
    })
}?.expect("Failed to initialize Atlas-Common");

// Keep the guard in scope for the lifetime of your application
```

## 🎛️ Feature Flags

<details>
<summary><strong>🌟 Complete Feature Reference</strong></summary>

### Async Runtime (choose one)
- `async_runtime_tokio` - Tokio runtime ⭐ **default**
- `async_runtime_async_std` - async-std runtime

### Thread Pools (choose one)
- `threadpool_rayon` - Rayon work-stealing pool ⭐ **default**
- `threadpool_crossbeam` - Crossbeam channel-based pool

### Network Sockets (choose one)
- `socket_tokio_tcp` - Tokio TCP sockets ⭐ **default**
- `socket_async_std_tcp` - async-std TCP sockets
- `socket_rio_tcp` - Rio TCP sockets

### Channels
- `channel_flume_mpmc` - Flume MPMC channels ⭐ **default**
- `channel_sync_crossbeam` - Crossbeam sync channels ⭐ **default**
- `channel_mixed_flume` - Flume mixed channels ⭐ **default**
- `channel_mult_custom_dump` - Custom dump channels ⭐ **default**
- `channel_async_channel_mpmc` - async-channel MPMC

### Cryptography
#### Signatures
- `crypto_signature_ring_ed25519` - Ring Ed25519 ⭐ **default**

#### Hashing
- `crypto_hash_blake3_blake3` - Blake3 hashing ⭐ **default**
- `crypto_hash_ring_sha2` - Ring SHA2 hashing

### Collections
- `collections_randomstate_fxhash` - FxHash random state ⭐ **default**
- `collections_randomstate_twox_hash` - TwoX hash random state
- `collections_randomstate_std` - Standard library random state

### Persistent Storage
- `persistent_db_sled` - Sled database ⭐ **default**
- `persistent_db_rocksdb` - RocksDB database

### Serialization
- `serialize_serde` - Serde serialization ⭐ **default**

</details>

## 🏗️ Architecture

Atlas-Common is designed with modularity at its core. Each major component can be swapped out through feature flags, allowing you to:

- **🎯 Optimize for your use case** - Choose the best-performing backend for your specific requirements
- **🔧 Customize for deployment** - Different configurations for development, testing, and production
- **🛡️ Enhance security** - Mix different implementations across nodes in a BFT network
- **⚖️ Balance trade-offs** - Performance vs. memory usage vs. compatibility

## 🤝 Integration with Atlas Framework

Atlas-Common serves as the foundation for all other Atlas modules:

- **🏛️ Atlas-Core** - Consensus protocols and core BFT logic
- **🔄 Atlas-SMR** - State machine replication
- **📡 Atlas-Communication** - Network communication layer
- **💾 Atlas-Persistent-Log** - Persistent logging and recovery
- **📊 Atlas-Metrics** - Performance monitoring and metrics

## 🔬 Advanced Features

### Threshold Cryptography
Built-in support for distributed cryptographic operations:
- **DKG (Distributed Key Generation)** - Secure key generation across multiple parties
- **Threshold Signatures** - Signatures requiring cooperation of multiple parties
- **FROST Protocol** - Flexible Round-Optimized Schnorr Threshold signatures

### Circuit Breaker Pattern
Prevent cascading failures in distributed systems with configurable circuit breakers.

### Memory Management
Optimized memory pools and allocation strategies for high-performance distributed computing.

## 📊 Benchmarks

The project includes comprehensive benchmarks:
```bash
cargo bench
```

## 🧪 Testing

Run the test suite:
```bash
cargo test
```

## 📄 License

This project is licensed under the MIT License - see the [LICENSE.txt](../LICENSE.txt) file for details.

## 👥 Authors

- **Nuno Neto** - *Lead Developer* - [nuno.martingo@fc.up.pt](mailto:nuno.martingo@fc.up.pt)

## 🔗 Links

- **🏠 Homepage**: [https://github.com/nuno1212s/atlas](https://github.com/nuno1212s/atlas)
- **📚 Documentation**: [https://docs.rs/atlas](https://docs.rs/atlas)
- **🐛 Issues**: [GitHub Issues](https://github.com/nuno1212s/atlas/issues)

---

<div align="center">

**Built with ❤️ for the distributed systems community**

</div>
