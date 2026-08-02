# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
# Build this crate
cargo build -p atlas-smr-execution

# Run tests
cargo test -p atlas-smr-execution
cargo test -p atlas-smr-execution test_name -- --nocapture

# Lint and format
cargo fmt --all
cargo clippy --all-targets --no-deps -- -D warnings
```

**Requirements:** Rust nightly (see `rust-toolchain.toml`), `capnproto` system dependency.

## Purpose

`atlas-smr-execution` sits between the consensus layer (`atlas-smr-core`) and the application (`atlas-smr-application`). It receives ordered `UpdateBatch` and unordered `UnorderedBatch` decisions from consensus, executes them against application state, and sends `BatchReplies` back to clients.

## Four Executor Variants

The crate provides four executor types across two axes:

| | MonolithicState | DivisibleState |
|---|---|---|
| **Single-threaded** | `SingleThreadedMonExecutor` | `SingleThreadedDivExecutor` |
| **Multi-threaded** | `MultiThreadedMonExecutor` | `MultiThreadedDivExecutor` |

- **MonolithicState**: Atomic, transferred in full during recovery. Simpler.
- **DivisibleState**: Partitionable; supports incremental state transfer via descriptor/parts API.
- **Multi-threaded**: Requires `CRUDState` + `CRUDApplication` traits; uses speculative parallel execution with collision detection.

All four implement `TMonolithicStateExecutor` or `TDivisibleStateExecutor` from `atlas-smr-core`.

## Speculative Execution Model (Multi-threaded path)

`src/scalable/mod.rs::scalable_execution` implements the core algorithm:

1. **Parallel phase** — Batch split into chunks; each thread runs operations through `ExecutionUnit` (`src/scalable/execution_unit/mod.rs`), which acts as a `CRUDState` proxy. It intercepts reads/writes into a local cache and records all accesses.
2. **Collision detection** — `CollisionState` tracks which keys were accessed and by whom. Write or delete on a key that another operation also accessed = collision.
3. **Sequential re-execution** — Collided operations re-run in order against the real state.
4. **Apply phase** — Non-collided speculative results merged back into actual state.

Collision rules: Write–anything and Delete–anything always collide. Read–Read never collides.

`ExecutionUnit v2` (`src/scalable/execution_unit/v2/`) is an alternative design not yet integrated.

Unordered batches always run fully parallel via `rayon::par_iter()` (no collision tracking needed).

## Channel Architecture

Three bounded channels connect the components:

- **Work channel** (`EXECUTING_BUFFER = 16384`): `ExecutionRequest` messages from consensus to executor worker.
- **State channel** (`STATE_BUFFER = 128`): State installation during recovery (`PollStateChannel`).
- **Checkpoint channel**: `AppStateMessage` flows from executor to the state transfer protocol on `UpdateAndGetAppstate` requests.

`ExecutionRequest` variants: `Update`, `UpdateAndGetAppstate`, `ExecuteUnordered`, `CatchUp`, `PollStateChannel`, `Read`.

## Reply Strategies (`src/repliers.rs`)

- `ReplicaReplier`: Sends replies for both ordered and unordered requests. Batches by client, dispatched in a thread pool.
- `FollowerReplier`: Only sends replies for unordered requests (followers skip ordered replies).

## Key Traits to Know

- **`CRUDState`** (`src/crud_states.rs`): `read`, `create`, `update`, `delete` keyed by `(column, key)`. Required for multi-threaded execution.
- **`CRUDApplication<S>`** (`src/crud_states.rs`): Extends `Application<S>` with `speculatively_execute()`. Required for scalable executors.
- **`Access` / `AccessType`**: Records per-operation data access patterns used by collision detection.

## Metrics

All metrics use IDs **800–806** (`src/metric/mod.rs`):
- 800 `EXECUTION_LATENCY_TIME_ID`, 801 `EXECUTION_TIME_TAKEN_ID`, 802 `REPLIES_SENT_TIME_ID`, 803 `REPLIES_PASSING_TIME_ID`
- 804 `OPERATIONS_EXECUTED_PER_SECOND_ID`, 805 `UNORDERED_OPS_PER_SECOND_ID`, 806 `UNORDERED_EXECUTION_TIME_TAKEN_ID`

## Thread Pool

Both multi-threaded executors use `rayon::ThreadPoolBuilder` with `THREAD_POOL_THREADS = 4` threads.