# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## This Crate

`atlas-common` is the foundational layer of the Atlas BFT framework. All other Atlas crates depend on it. It provides pluggable backends for async runtime, thread pools, channels, crypto, sockets, and persistent storage — selected at compile time via Cargo feature flags.

## Build and Test Commands

```bash
# Build (from this directory or workspace root)
cargo build
cargo build --release

# Run all tests for this crate
cargo test -p atlas-common
cargo test -p atlas-common test_name -- --nocapture

# Benchmarks
cargo bench

# Lint
cargo clippy --all-targets --no-deps -- -D warnings
cargo fmt --all -- --check
```

**Requirements:** Rust nightly (see `rust-toolchain.toml`).

## Feature Flags Architecture

The entire crate is structured around mutually exclusive feature groups. Each major subsystem has one active backend chosen at compile time:

| Group | Default | Alternatives |
|---|---|---|
| `async_runtime_*` | `tokio` | `async_std` |
| `threadpool_*` | `rayon` | `crossbeam` |
| `socket_*` | `tokio_tcp` | `async_std_tcp`, `rio_tcp` |
| `crypto_signature_*` | `ring_ed25519` | — |
| `crypto_hash_*` | `blake3_blake3` | `ring_sha2` |
| `channel_*` | `flume_mpmc` + `sync_crossbeam` + `mixed_flume` + `mult_custom_dump` | various |
| `collections_randomstate_*` | `fxhash` | `twox_hash`, `std`, `gxhash` |
| `persistent_db_*` | `sled` | `rocksdb`, disabled |
| `serialize_*` | `serde` | `capnp` (broken) |

When adding a new backend, follow the pattern: create `src/<module>/<backend_name>/mod.rs`, gate everything with `#[cfg(feature = "...")]`, and re-export from `src/<module>/mod.rs`.

## Key Data Structures

**`SeqNo`** (`src/ordering/mod.rs`) — Core sequence number type used throughout Atlas. Wraps `i32`, implements wrap-around arithmetic. `SeqNo::index()` returns `Either<InvalidSeqNo, usize>` — messages too far ahead are rejected (DoS protection, threshold is `PERIOD + PERIOD/2 = 75000`).

**`TboQueue`** (`src/ordering/tbo_queue/`) — "To-Be-Ordered" queue. Buffers out-of-order messages indexed by sequence number offset from current. Two implementations: `VecTboQueue` and `BTreeTboQueue`.

**`SingularTboQueue`** (`src/ordering/singular_tbo_queue/`) — Like `TboQueue` but stores at most one message per sequence slot.

**`MaybeVec<T>`** (`src/maybe_vec/`) — Stack-allocated enum (`None | One(T) | Mult(Vec<T>)`) to avoid heap allocation when typically holding 0–1 items.

## Initialization Pattern

All consumers must call `atlas_common::init(InitConfig { async_threads, threadpool_threads })` before using any runtime-backed functionality. Returns an `InitGuard` that teardowns resources on drop. The guard must stay in scope for the program's lifetime.

## Channel Abstractions

`src/channel/mod.rs` re-exports four channel flavors:
- `channel::async` — async MPMC (flume or async-channel)
- `channel::sync` — sync MPMC (crossbeam or flume)
- `channel::mixed` — tx is sync, rx is async (or vice versa)
- `channel::mult` — multi-dump channel for batching (dsrust)
- `channel::oneshot` — single-use channel

All backends implement the same interface so call sites are feature-agnostic.

## Crypto Modules

`src/crypto/` has three subsystems:
- `signature/` — Ed25519 signing/verification (`ring`)
- `hash/` — Fast digests (`blake3` or `ring sha2`)
- `threshold_crypto/` — Threshold signatures via `threshold_crypto` crate (BLS-based DKG + signing) and FROST Ed25519