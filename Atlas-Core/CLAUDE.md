# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Test Commands

```bash
# Build this crate
cargo build -p atlas-core

# Run all tests
cargo test -p atlas-core
cargo nextest run -p atlas-core

# Run a specific test with output
cargo test -p atlas-core <test_name> -- --nocapture

# Lint
cargo clippy -p atlas-core --no-deps -- -D warnings
cargo fmt --all -- --check
```

**Requirements:** Rust nightly (see `rust-toolchain.toml` at workspace root). Crate features: `serialize_serde` (enables serde) and `serialize_capnp` (enables Cap'n Proto via `atlas-capnp`).

## Crate Purpose

`atlas-core` is the **trait-definition layer** for Atlas BFT/CFT SMR. It defines the abstract interfaces that all protocol implementations must satisfy. It contains no concrete protocol logic — only types, traits, and glue.

Dependencies: `atlas-common` (utilities), `atlas-communication` (network message types), `atlas-metrics`.

## Module Architecture

### `ordering_protocol/` — Central abstraction

The most important module. Key traits:

- **`OrderingProtocol<RQ>`** — The core protocol trait. Implementors (e.g. FeBFT) must provide `poll()` → `OPPollResult` and `process_message()` → `OPExecResult`. The protocol is driven by a run-loop that alternates between these two methods.
- **`OrderProtocolTolerance`** — Quorum math (`get_n_for_f`, `get_quorum_for_n`, `get_f_for_n`).
- **`PermissionedOrderingProtocol`** — Extension for protocols where only a subset of nodes vote.
- **`OrderingProtocolArgs<R, RQPP, NT>`** — Tuple struct bundling initialization arguments passed to protocol constructors.

Type aliases in `mod.rs` reduce generic noise:
- `ShareableConsensusMessage<RQ, OP>` = `Arc<StoredMessage<OP::ProtocolMessage>>`
- `OPResult<RQ, SER>` / `OPExResult<RQ, SER>` — poll and exec return types

#### `ordering_protocol/decision.rs`

Tracks the lifecycle of a single consensus decision through `Decision<MD, DAD, PM, RQ>`. A decision accumulates `DecisionPart` variants in order:
1. `DecisionMetadata` — proof metadata
2. `PartialDecisionInformation` — intermediate protocol messages
3. `DecisionRequests` — the actual client request batch
4. `DecisionDone` — signals completion

`DecisionPart` ordering is enforced via `Ord` (rank 0–3). The `MaybeOrderedVec` from `atlas-common` holds parts without unnecessary allocation. `Decision::merge_decisions()` combines updates for the same seq number.

`DecisionRequests<O>` wraps the `DecisionRequestBatch<O>` (executable batch) plus `Vec<ClientRqInfo>` (dedup tracking) and a batch digest.

#### `ordering_protocol/networking/serialize/`

Trait definitions for protocol message types:
- **`OrderingProtocolMessage<RQ>`** — Associated types `ProtocolMessage`, `DecisionMetadata`, `DecisionAdditionalInfo`. All must implement `Orderable + SerMsg`.
- **`PermissionedOrderingProtocolMessage`** — Associated `ViewInfo: NetworkView + SerMsg`.
- **`NetworkView`** — Quorum membership: `primary()`, `quorum()`, `quorum_members()`, `f()`, `n()`.
- **`OrderProtocolVerificationHelper`** — Signature verification callbacks for request and protocol messages.

#### `ordering_protocol/loggable/`

Extends `OrderingProtocol` with log-writeable proofs via `PProof` associated type. Used by `atlas-logging-core`.

#### `ordering_protocol/reconfigurable_order_protocol/`

Trait for protocols that support dynamic membership changes.

### `execution/`

- **`TExecutorDecisionHandle<RQ>`** — Base handle: `catch_up_to_quorum()`, `queue_update_unordered()`.
- **`TDeterministicExecutorDecisionHandle<RQ>`** — Adds `queue_update()` for ordered batches.
- **`TPreemptiveExecutorDecisionHandle<RQ>`** — Adds `queue_preemptive_update()` and `queue_preemptive_update_finalized()` for the preemptive execution branch (currently in development on `preemptive_execution` branch).

The handle is a channel abstraction — implementations live in `atlas-smr-execution`.

### `persistent_log/`

- **`OrderingProtocolLog<RQ, OP>`** — Write protocol state durably: committed seq no, messages, metadata, additional data, invalidation. `OperationMode` controls blocking vs. non-blocking writes.
- **`PermissionedOrderingProtocolLog<POP>`** — Persists view state.

### `timeouts/`

Multi-worker timeout system. `TimeoutsHandle` dispatches to worker threads via channels.

- **`TimeoutID`** — `SeqNoBased(SeqNo)` for consensus rounds; `SessionBased { session, seq_no, from }` for client request tracking. Routing: seq-based always goes to worker 0; session-based is sharded by `operation_key_raw(from, session) % num_workers`.
- **`TimeoutIdentification`** — `(mod_id: Arc<str>, TimeoutID)` — identifies which module owns the timeout.
- **`TimeoutableMod<R>`** (in `timeout/`) — Trait for modules that process timeouts; `TimeoutModHandle` is the send-side handle.
- **`TimeoutWorkerResponder`** — Callback for delivering fired timeouts back to the system.

### `request_pre_processing/`

Traits for the request pre-processor that sits between clients and the ordering protocol:
- **`WorkPartitioner`** — Routes client sessions to workers. Contract: a given `(client_id, session)` pair always routes to the same worker.
- **`RequestPreProcessing<O>`** — Process forwarded requests, stopped requests, and decided batches.
- **`BatchOutput<O>`** — Typed receive-side wrapper around the channel carrying deduplicated batches from the pre-processor to the ordering protocol.
- **`operation_key_raw(from, session)`** — Packs a `(NodeId, SeqNo)` into a `u64` key used for sharding and dedup.

### `messages/`

`ClientRqInfo`, `ForwardedRequestsMessage`, and `SessionBased` trait (used for session-based routing).

### `reconfiguration_protocol/`, `followers/`

Traits for view change / reconfiguration and follower-mode support (non-voting replicas).

## Key Design Invariants

- **`Decision` parts must arrive in rank order** (`DecisionMetadata` < `PartialDecisionInformation` < `DecisionRequests` < `DecisionDone`). `MaybeOrderedVec` enforces this at insertion.
- **`WorkPartitioner` must be deterministic and sticky per session**: all messages for a `(client, session)` must map to the same worker, or dedup breaks.
- **`OperationMode::BlockingSync`** stalls the calling thread; prefer `NonBlockingSync` on the hot path unless ordering guarantees are required.
- **`ShareableConsensusMessage` is `Arc`-wrapped**: use `unwrap_shareable_message()` to avoid cloning when you hold the last reference.
