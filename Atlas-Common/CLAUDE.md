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

Each major subsystem has one backend chosen at compile time. **There is no backend
in `default`** — the default is selected by the *absence* of an override flag:

| Group | Default (no flag) | Override with |
|---|---|---|
| async runtime | tokio | `async_runtime_async_std` |
| socket | tokio TCP | `socket_async_std_tcp`, `socket_rio_tcp` |
| threadpool | rayon | `threadpool_crossbeam` |
| hash | blake3 | `crypto_hash_ring_sha2` |
| async channel | flume | `channel_async_channel_mpmc` |
| sync channel | crossbeam | `channel_sync_flume` |
| dump queue | mqueue | `channel_custom_dump_lfb` |
| `RandomState` | fxhash | `collections_randomstate_{std,twox_hash,gxhash}` |
| persistent db | sled | `persistent_db_rocksdb`, `persistent_db_disabled` |

Signature (`ring` Ed25519), the mixed channel and the multi-dump channel have a
single implementation each and carry no flag.

Why the inversion: Cargo features are additive and unioned graph-wide, so a
positive default could only be turned off with `default-features = false` on
every one of the ~20 edges that reach this crate — miss one and the union
restores it. With the default expressed as `not(any(<overrides>))`, enabling one
flag *anywhere* in the graph (including from the leaf binary) switches that group.

The cost of the inversion: each group's default backend must be a **non-optional**
dependency, because Cargo cannot enable an optional dependency from the absence of
a feature — not from `[features]`, and not from a build script either.

`serialize_serde` is the one exception and stays a positive default feature: it
is a cross-cutting derive gate, not a backend choice, and ~12 sibling crates
forward it.

`build.rs` rejects two overrides from the same group with a readable message.
It also catches the genuinely ambiguous case where two different crates in the
graph each pick a different backend.

When adding a new backend: create `src/<module>/<backend>/mod.rs` exposing the
same surface as the existing one, gate it on a new override feature, and add that
feature to the group's `not(any(...))` predicate *and* to `build.rs`'s `GROUPS`.

### Backend caveats

- `collections_randomstate_gxhash` needs `RUSTFLAGS="-C target-feature=+aes,+sse2"`
  (or `-C target-cpu=native`); gxhash itself refuses to compile otherwise.
- `persistent_db_rocksdb` needs libclang and the C standard headers for
  `librocksdb-sys`' bindgen step.

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