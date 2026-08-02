# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Crate Role

`atlas-smr-core` is the **bridging layer** between the generic consensus engine (`atlas-core`) and the SMR application layer (`atlas-smr-application`). It does not implement consensus — it wraps, adapts, and routes.

Key dependency stack (bottom → top):
```
atlas-common → atlas-communication → atlas-core
                                          ↓
atlas-smr-application → atlas-smr-core ←←←
                                          ↓
          atlas-smr-execution / atlas-smr-replica
```

## Module Responsibilities

- **`execution/`** — Wraps `atlas-core`'s decision output into SMR `UpdateBatch`/`UnorderedUpdateBatch`. `SMRExecWrapper` implements `atlas-core`'s executor traits; `TExecutorStateHandle` variants (deterministic, preemptive) manage state handles. Executors in `executors/` handle monolithic vs divisible state.

- **`message/`** — Defines `OrderableMessage<D>` (application-level: ordered/unordered requests and replies) and `SystemMessage<D,P,LT,VT>` (system-level: protocol, log/view transfer, forwarded requests). These are the canonical message enums for the replica.

- **`networking/`** — `SMRReplicaNetworkNode` trait defines the full replica networking interface via four associated types: `ProtocolNode`, `ApplicationNode`, `StateTransferNode`, `ReconfigurationNode`. `ReplicaNodeWrapper` is the concrete implementation. `ReplyNode<RP>` trait handles sending replies to clients (send/broadcast, signed/unsigned).

- **`persistent_log/`** — `MonolithicStateLog` and `DivisibleStateLog` traits for state snapshot persistence.

- **`request_pre_processing/`** — `RequestPreProcessor` drives batching and forwarding before consensus. Worker thread processes `PreProcessorMessage` variants. Has unit tests under `tests/`.

- **`serialize/`** — `Service<D,P,L,VT>` implements `Serializable` by delegating to protocol/log/view-transfer verifiers. Entry point for message serialization and signature verification.

- **`state_transfer/`** — `StateTransferProtocol<S>` trait for state sync. `Checkpoint<S>` holds a state snapshot with a sequence number and digest. Submodules split monolithic vs divisible state transfer and their respective networking traits.

- **`metric/`** — Metric IDs for request preprocessing instrumentation (registered with `atlas-metrics`).

## Key Type Aliases

```rust
// lib.rs
pub type SMRReq<D>  = RequestMessage<D::Request>;
pub type SMRRawReq<R> = RequestMessage<R>;
pub type SMRReply<D> = ReplyMessage<D::Reply>;
```

## Request Flow (end-to-end)

1. Clients → `ApplicationNode::receive_from_clients()`
2. `RequestPreProcessor` batches/forwards requests
3. `ProtocolNode` feeds batches into consensus (`atlas-core`)
4. `SMRExecWrapper` adapts ordered decisions → `UpdateBatch`
5. Application executes; replies sent via `ReplyNode`

## State Transfer Modes

- **Monolithic** — single atomic snapshot (`MonolithicStateTransfer`, `MonolithicStateLog`)
- **Divisible** — partitioned state with descriptors and parts (`DivisibleStateTransfer`, `DivisibleStateLog`)

Choose based on whether application state can be transferred incrementally.

## Generics Convention

Most types are parameterized as `<D, P, L, VT, S>`:
- `D: ApplicationData` — request/reply types
- `P` — ordering protocol message type
- `L` / `LT` — log transfer message type
- `VT` — view transfer message type
- `S` — state type (implements `MonolithicState` or `DivisibleState`)

Type errors in `atlas-smr-replica` often originate here due to mismatched generic bounds.

## Features

- `serialize_serde` (default) — Serde-based serialization
- `serialize_capnp` — Cap'n Proto serialization (requires `atlas-capnp`)