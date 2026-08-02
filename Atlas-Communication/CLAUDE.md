# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
# Build
cargo build
cargo build --release

# Run all tests
cargo nextest run           # preferred (parallel)
cargo test                  # standard

# Run a single test with output
cargo test test_name -- --nocapture

# Lint and format
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --no-deps -- -D warnings
```

**Requirements:**
- Rust nightly (pinned in `rust-toolchain.toml`)
- Optional: `atlas-capnp` system dependency when using `serialize_capnp` feature

**Feature flags:**
- `serialize_serde` (default) — bincode/serde serialization
- `serialize_capnp` — Cap'n Proto serialization (requires `atlas-capnp`)

## Architecture Overview

`atlas-communication` implements the upper three layers of Atlas's 4-layer network abstraction. The fourth (byte/transport) layer is implemented externally (e.g., `atlas-comm-mio`) and injected via traits.

### Layer Stack

```
┌─────────────────────────────────────────┐
│  Application Layer — Protocol Stubs     │
│  ReconfigurationStub, OperationStub,    │
│  StateProtocolStub, ApplicationStub     │
├─────────────────────────────────────────┤
│  Message Layer — Serialization          │
│  Signing, verification, routing         │
├─────────────────────────────────────────┤
│  Connection Layer — Peer Management     │
│  PeerConnectionManager, routing table   │
├─────────────────────────────────────────┤
│  Byte Layer (external crate)            │
│  ByteNetworkController / ByteNetworkStub│
└─────────────────────────────────────────┘
```

### Central Coordinator

`NetworkManagement<NI, CN, BN, R, O, S, A>` (in `src/lib.rs`) is the top-level type. Its generic parameters:

| Param | Meaning |
|-------|---------|
| `NI` | Network information / topology provider |
| `CN` | Byte network stub (per-connection handle) |
| `BN` | Byte network controller (factory) |
| `R, O, S, A` | Message types: Reconfiguration, Operation/Protocol, StateProtocol, Application |

Entry points exposed:
- `NetworkManagement::initialize(...)` — creates the manager
- `init_op_stub()`, `init_reconf_stub()`, `init_state_stub()`, `init_app_stub()` — returns typed stubs for each protocol layer

### Message Routing

`EnumLookupTable<R,O,S,A>` (`src/lookup_table/`) provides O(1) dispatch via `enum_map!`. The `MessageModule` enum has four variants (Reconfiguration, Protocol, StateProtocol, Application). Each variant routes to the correct serialization/deserialization handler. `ModMessageWrapped<R,O,S,A>` is the enum that discriminates message types at runtime.

### Key Traits

**Serialization** (`src/serialization/`):
- `Serializable` — associated `Message` type + `Verifier` for verification
- `InternalMessageVerifier<M>` — verifies incoming messages given network info

**Outgoing/Incoming stubs** (`src/stub/`):
- `ModuleOutgoingStub<M>` — `send`, `send_signed`, `broadcast`, `broadcast_signed`
- `ModuleIncomingStub<M>` — `receive_messages`, `try_receive_messages`

**Byte layer contracts** (`src/byte_stub/`):
- `ByteNetworkStub` — `dispatch_blocking(WireMessage)` — send raw bytes
- `NodeStubController` — manage per-peer stubs (create, lookup, shutdown)
- `NetworkStub<T>` / `RegularNetworkStub<T>` — higher-level per-module network handles

### Wire Message Format

`Header` is a fixed 128-byte `#[repr(C, packed)]` struct:
- `from`/`to`: `u32` node IDs
- `nonce`: `u64`
- `length`: `u64` payload length
- `digest`: 32-byte SHA-256
- `signature`: 64-byte Ed25519

### Message Processing Flow

**Outgoing**: Serialize payload → compute digest → optionally sign → wrap in `WireMessage` → `ByteNetworkStub::dispatch_blocking`

**Incoming**: Receive `WireMessage` → verify auth (unauthenticated peers only accepted for Reconfiguration messages) → deserialize → route via `EnumLookupTable` → deliver to stub channel

### Authentication

Peers start unauthenticated. Only Reconfiguration messages are accepted from unauthenticated peers. Once authenticated, all four message modules are accepted. This is enforced in `src/byte_stub/incoming/` processing.

## Testing

Integration tests use mock implementations in `tests/integration_testing.rs`:
- `MockNetworkInfo` — simulates topology
- `MockByteStub` / `MockByteController` — channel-based fake transport

When adding new message modules or serialization paths, add corresponding mock variants here.