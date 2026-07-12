# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# Project

Atlas is a modular BFT/CFT framework to develop applications which are naturally fault tolerant, as well as allowing for the easier development of new individual protocols, such as consensus protocols, state transfer protocols, network protocols, etc.

With Atlas having very well defined interfaces to serve as bounds for each of the modules, the users are free to implement their own protocols and applications without having to redevelop the wheel in the process.

## Build and Development Commands

```bash
# Build workspace
cargo build
cargo build --release

# Run all tests
cargo test
cargo nextest run         # faster parallel test runner

# Run tests for a specific crate
cargo test -p atlas-smr-core
cargo test -p atlas-common test_name -- --nocapture

# Lint and format
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --no-deps -- -D warnings
```

**Requirements:**
- Rust nightly (pinned in `rust-toolchain.toml`)
- `capnproto` system dependency (required by `atlas-capnp`)

## Architecture Overview

Atlas is a modular Byzantine Fault Tolerant (BFT) and Crash Fault Tolerant (CFT) State Machine Replication (SMR) framework. The workspace has ~17 crates organized in layers:

### Layer 1: Foundation
- **atlas-common** — Core utilities and pluggable backends (async runtime, thread pools, crypto, channels, serialization, storage). Feature flags select between implementations (e.g., `tokio` vs `async-std`, `sled` vs `rocksdb`, `ring` for Ed25519 signatures).
- **atlas-capnp** — Cap'n Proto schema definitions; `build.rs` compiles `.capnp` schemas.
- **atlas-metrics** — Metrics collection and performance monitoring.

### Layer 2: Protocol Core
- **atlas-core** — BFT/CFT protocol abstractions: ordering protocols, timeout management, request pre-processing, persistent log abstraction, reconfiguration.
- **atlas-communication** — Upper 3 layers of the network abstraction:
  1. Application layer: typed stubs (`OperationStub`, `ReconfigurationStub`, `StateProtocolStub`, `ApplicationStub`)
  2. Message layer: serialization, verification, routing
  3. Connection layer: `PeerConnectionManager`, `NetworkNode`
- **atlas-comm-mio** — Implements the byte layer (raw wire protocol/I/O) on top of which `atlas-communication` builds.
- **atlas-reconfiguration** — View change and system reconfiguration.

### Layer 3: Logging and State Transfer
- **atlas-logging-core** / **atlas-decision-log** / **atlas-persistent-log** — Consensus decision logging abstractions and implementations.
- **atlas-log-transfer** — Log transfer protocol between replicas.
- **atlas-view-transfer** — View transfer protocol for view changes.

### Layer 4: SMR Stack
- **atlas-smr-core** — SMR message types and core abstractions.
- **atlas-smr-application** — User-facing traits:
  - `ApplicationData`: defines `Request` and `Reply` associated types.
  - `Application`: implement `initial_state()`, `update()`, `unordered_execution()`.
- **atlas-smr-execution** — Execution layer; processes ordered decisions and dispatches to the application. Supports standard and batch execution.
- **atlas-smr-preemptive-execution** — Preemptive execution variant (currently in development on the `preemptive_execution` branch).
- **atlas-smr-replica** — Central orchestrator: wires together ordering protocol, state transfer, execution, log, and communication components. Heavily generic over all component types.
- **atlas-client** — Client implementation for submitting requests to replicas.

### Submodules
- **Atlas-Examples** — Example applications (e.g., Calculator SMR app showing `Application`/`ApplicationData` trait usage).
- **Atlas-Tools** — Key generation and default configuration utilities.

## Key Design Patterns

**Feature-flag backends:** `atlas-common` gates most infrastructure choices behind Cargo features. When adding new backend support or debugging build issues, check feature flags and conditional compilation blocks.

**Heavy generics:** The codebase uses Rust generics and associated types extensively. `atlas-smr-replica` is generic over ordering protocol, state transfer, execution, log, and crypto types — so type errors often manifest there even when the root cause is in another crate.

**Trait-based protocol extensibility:** Ordering protocols implement traits defined in `atlas-core`. FeBFT is the reference consensus implementation (external crate).

**Cap'n Proto schemas:** Schema files are in `atlas-capnp/`. After modifying schemas, `cargo build` re-runs the `build.rs` compiler. Generated code lands in `target/`.