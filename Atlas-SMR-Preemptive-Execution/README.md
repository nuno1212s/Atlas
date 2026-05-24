# Atlas-SMR-Preemptive-Execution

Speculative execution layer for the Atlas BFT/CFT SMR framework. Requests are executed *before* consensus finalizes them so that client replies can be sent earlier, hiding ordering latency. Two independent executor designs are provided, each with different memory and compute trade-offs.

---

## Table of Contents

- [Why preemptive execution?](#why-preemptive-execution)
- [Shared message protocol](#shared-message-protocol)
- [Executor 1: Dual-State](#executor-1-dual-state)
- [Executor 2: CRUD Cache](#executor-2-crud-cache)
- [Choosing between the two](#choosing-between-the-two)
- [Metrics](#metrics)
- [Testing](#testing)

---

## Why preemptive execution?

In a conventional SMR replica the execution pipeline is:

```
Client request → Ordering protocol → Execute → Reply to client
```

The time the ordering protocol takes (consensus rounds, network RTTs) sits directly on the critical path of client-visible latency. Preemptive execution breaks this dependency by beginning execution as soon as a request is *proposed*, before consensus has finished:

```
Client request → Ordering protocol → Finalize (commit/abort)
                        ↓                    ↓
                 Execute speculatively   Commit replies  ←── much earlier
                 (on proposed order)    or discard
```

If the speculation turns out to be correct (the proposed order matches the final order) the client reply is ready the instant consensus completes. If speculation was wrong (a backtrack occurs) the speculative work is discarded and re-executed on the correct order.

---

## Shared message protocol

Both executors receive the same `PreemptiveExecutionRequest<O>` message type via a `PreemptiveExecutorHandle<O>`. The handle implements three traits from `atlas-smr-application`:

| Trait | Methods | Purpose |
|---|---|---|
| `TExecutionHandle` | `poll_state_channel`, `catch_up_to_quorum`, `queue_unordered` | State transfer, catch-up, read-only queries |
| `TDeterministicExecutionHandle` | `queue_update`, `queue_update_and_get_appstate` | Directly-confirmed updates (no prior speculation) |
| `TPreemptiveExecutionHandle` | `queue_preemptive_update`, `queue_update_finalized`, `queue_update_finalized_and_get_appstate` | Speculative updates and their finalization |

### Request variants

| Variant | Meaning |
|---|---|
| `PreemptiveUpdate(batch)` | Execute speculatively; store replies until confirmation |
| `PreemptiveUpdateFinalized(seq)` | Confirm the pending update at `seq`; send pre-computed replies |
| `PreemptiveUpdateFinalizedAndGetAppstate(seq)` | Same as above, also emit a confirmed-state checkpoint |
| `UpdateBatch(batch)` | Execute directly on confirmed state (no prior speculation for this seq) |
| `UpdateBatchAndGetAppstate(batch)` | Same, also emit checkpoint |
| `CatchUp(batches)` | Apply multiple batches directly; discards all pending speculation |
| `PollStateChannel` | Switch to state-transfer mode; next message installs a new state snapshot |
| `ExecuteUnordered(batch)` | Read-only execution on confirmed state via a rayon thread pool |

Checkpoints (`AppStateMessage<S>`) are received on a separate channel returned by the executor's `init` function (`MonStateInstallHandle<S>`). The same handle carries the state-install sender used during state transfer.

---

## Executor 1: Dual-State

**Source:** `src/single_thread_double_state/`

**Selector:** `MonolithicPreemptiveExecutor` in `src/lib.rs`

**Trait requirements:** `S: MonolithicState`, `A: Application<S>`

### Design

Maintains two full copies of the application state: a *preemptive* (speculative) copy and a *confirmed* (authoritative) copy. Two dedicated worker threads operate on them in parallel.

```
┌─────────────────────────────────────────────────────────┐
│  Orchestrator thread                                      │
│  Receives PreemptiveExecutionRequest, routes messages     │
└──────────────┬──────────────────────────┬────────────────┘
               │                          │
               ▼                          ▼
┌──────────────────────────┐  ┌──────────────────────────┐
│  Preemptive Worker       │  │  Confirmed Worker        │
│  state: preemptive_state │  │  state: confirmed_state  │
│  queue: VSingleTBOQueue  │  │  re-executes each batch  │
│  (batch, replies) pairs  │  │  sends replies to client │
└──────────────────────────┘  └──────────────────────────┘
         │  PreemptiveToConfirmedMsg (finalized batch)
         └──────────────────────────────────────────►
                                  confirmed_worker re-runs it
```

### Operation

**Preemptive update at seq N:**
1. Execute on `preemptive_state`; store `(batch, replies)` in the TBO queue.
2. Advance `preemptive_seq_no`.

**Finalization at seq N:**
1. Pop the entry from the TBO queue.
2. Forward the batch to the confirmed worker via `PreemptiveToConfirmedMsg`.
3. The confirmed worker **re-executes** the batch on `confirmed_state` and sends replies to clients.

**Backtrack:**
1. Drain the TBO queue.
2. Request a state snapshot from the confirmed worker.
3. Replace `preemptive_state` with the snapshot; discard all pending speculative work.

### Trade-offs

- Requires two full state copies — 2× memory usage.
- The confirmed worker re-executes every batch — 2× CPU for ordered execution.
- Backtracking requires a full state clone from the confirmed worker.
- Does not require any special state interface (`CRUDState`).

### Key files

| File | Purpose |
|---|---|
| `mod.rs` | Orchestrator: routes requests to the appropriate worker |
| `preemptive_worker/preemptive_requests.rs` | `PreemptiveRequestPipeline` — TBO queue, speculative execution |
| `preemptive_worker/mod.rs` | Preemptive worker event loop |
| `confirmed_worker/confirmed_requests.rs` | `ConfirmedRequestPipeline` — confirmed state, re-execution |
| `confirmed_worker/mod.rs` | Confirmed worker event loop |
| `state_management.rs` | Inter-worker message types (state snapshot request/response) |
| `comm_handles.rs` | Shared channel type aliases |

---

## Executor 2: CRUD Cache

**Source:** `src/single_threaded_crud/`

**Selector:** `CRUDMonolithicPreemptiveExecutor` in `src/lib.rs`

**Trait requirements:** `S: MonolithicState + CRUDState + Sync`, `A: CRUDApplication<S>`

### Design

Maintains a **single confirmed state** plus an in-memory **accumulated cache** of pending speculative writes. A single worker thread handles all requests sequentially.

```
Confirmed state  ←── only written on confirmation
      │
      │  read fallback
      ▼
Accumulated cache  ←── merge of all pending deltas (latest write wins per key)
      │
      │  read fallback
      ▼
Per-update local delta  ←── writes from the current speculatively executing batch
```

Each preemptive update executes through a `CachingState<'a, S>` proxy that implements `CRUDState`:

- **Reads:** check local delta first, then the accumulated cache, then the real state.
- **Writes:** go into the local delta only.
- **Deletes:** stored as `None` tombstones in the delta so subsequent reads see the key as absent.

On confirmation: apply the pre-computed delta to the real state, rebuild the accumulated cache from the remaining pending deltas, send the pre-computed replies. **No re-execution.**

On backtrack: discard pending entries at or after the backtrack seq, rebuild the accumulated cache from the kept entries, reset `preemptive_seq_no`. **No state clone.**

### Operation

**Preemptive update at seq N** (`handle_preemptive_update`):
1. Validate `seq == preemptive_seq_no + 1`; return `Backtracking` error if behind, `FutureRequest` if more than one ahead.
2. Create `CachingState { confirmed_state, accumulated_cache, delta: empty }`.
3. Execute all ops via `application.speculatively_execute(&mut caching_state, op)` and collect replies.
4. Merge the local delta into the accumulated cache.
5. Push `PendingCachedUpdate { batch, replies, delta }` onto the pending queue.

**Confirmation at seq N** (`handle_update_confirmed`):
1. Pop the head entry (must match `seq`).
2. Apply its delta to `confirmed_state` (writes → `update`, tombstones → `delete`).
3. Rebuild the accumulated cache from the remaining pending deltas.
4. Return the pre-computed replies.

**Backtrack to seq B** (`backtrack`):
1. Retain only pending entries with `seq < B`; discard the rest.
2. Rebuild the accumulated cache from the kept deltas.
3. Reset `preemptive_seq_no` to the last kept entry's seq (or `confirmed_seq_no` if none).

**Direct confirmed update** (`handle_confirmed_update`):
- Requires `preemptive_seq_no == confirmed_seq_no` (no pending speculation).
- Executes directly on `confirmed_state` via `application.update_batch`.

**CatchUp** (`handle_catch_up`):
- Clears the pending queue and the accumulated cache.
- Applies each batch directly to `confirmed_state` and advances both seq counters.

**State transfer** (`install_confirmed_state`):
- Replaces the confirmed state.
- Resets both seq counters; clears cache and pending queue.

### Key files

| File | Purpose |
|---|---|
| `mod.rs` | `CachingPreemptiveWorker` — single worker event loop; routes `PreemptiveExecutionRequest` variants |
| `caching_state.rs` | `CachingState<'a, S>` implementing `CRUDState` with layered read priority and tombstone semantics; `AccumulatedCache` type; `merge_delta_into`, `apply_delta_to_state`, `rebuild_accumulated_cache` helpers |
| `pending_state.rs` | `CachingPreemptiveState<S, A>` — core state machine; `PendingCachedUpdate`; `PreemptiveError`, `ConfirmError`, `BacktrackError` |
| `tests/test_fixtures.rs` | Shared test helpers: `MapState`, `MapApp`, `NoopNode`, `make_batch`, `spawn_worker` |
| `tests/unit_tests.rs` | Unit tests for `CachingPreemptiveState` — no channels, no threads |
| `tests/integration_tests.rs` | End-to-end tests via `PreemptiveExecutorHandle` + checkpoint receiver |

### Trade-offs

- Requires only one state copy — half the memory of the dual-state design.
- Confirmation is O(delta size), not O(batch size) — no re-execution.
- Backtracking is O(pending × delta size) for the cache rebuild — no state clone.
- Requires `S: CRUDState + Sync` and `A: CRUDApplication<S>`.
- Single-threaded: no parallelism between preemptive and confirmed processing.

---

## Choosing between the two

| Factor | Dual-State | CRUD Cache |
|---|---|---|
| Application trait | `Application<S>` only | `CRUDApplication<S>` + `CRUDState` |
| Memory overhead | 2× full state copies | 1× state + delta cache |
| Confirmation cost | Full re-execution | Apply pre-computed delta |
| Backtrack cost | Clone full confirmed state | Discard deltas, rebuild cache |
| Thread count | 3 (orchestrator + 2 workers) | 1 worker |
| Reply semantics | Sent from confirmed worker | Sent on confirmation by single worker |

**Use the dual-state executor** when the application state does not implement `CRUDState` or when you need the simplest possible correctness story.

**Use the CRUD cache executor** when memory footprint and re-execution overhead matter, and the application can express its state via the key-value CRUD interface.

---

## Metrics

All metrics are registered via `src/metric.rs` and emitted using `atlas-metrics`. Both executors are mutually exclusive alternatives, so their ID ranges may overlap in the same ID space without conflict.

### Dual-state executor metrics (800–807)

| ID | Name | Kind | Wired in | Description |
|---|---|---|---|---|
| 800 | `CONFIRMED_WORKER_LATENCY` | Duration | `confirmed_worker/mod.rs` | Time from batch arrival at the confirmed worker to actual execution |
| 801 | `CONFIRM_EXECUTION_TIME` | Duration | `confirmed_requests.rs` | Time for the confirmed worker to re-execute a batch on the authoritative state |
| 804 | `DS_PREEMPTIVE_EXECUTION_TIME` | Duration | `preemptive_requests.rs` | Time for the preemptive worker to speculatively execute a batch |
| 805 | `DS_SPECULATION_TO_CONFIRM_LATENCY` | Duration | `preemptive_requests.rs` | Time from speculative execution to consensus confirmation |
| 806 | `DS_BACKTRACK_COUNT` | Counter | `preemptive_worker/mod.rs` | Total number of backtrack events |
| 807 | `DS_OPS_PER_BATCH` | Count | `preemptive_requests.rs` | Number of operations per speculatively executed batch |

### CRUD cache executor metrics (802–803, 809–816)

| ID | Name | Kind | Wired in | Description |
|---|---|---|---|---|
| 802 | `CACHE_PREEMPTIVE_EXECUTION_TIME` | Duration | `pending_state.rs` | Time to speculatively execute a batch through `CachingState` |
| 803 | `CACHE_CONFIRM_APPLICATION_TIME` | Duration | `pending_state.rs` | Time to apply a pre-computed delta to the confirmed state |
| 809 | `CACHE_SPECULATION_TO_CONFIRM_LATENCY` | Duration | `pending_state.rs` | Time from speculative execution to consensus confirmation |
| 810 | `CACHE_PENDING_QUEUE_SIZE` | Count | `pending_state.rs` | Number of pending updates at the time a confirmation arrives |
| 811 | `CACHE_BACKTRACK_COUNT` | Counter | `pending_state.rs` | Total number of backtrack events |
| 812 | `CACHE_DELTA_SIZE` | Count | `pending_state.rs` | Number of K/V entries in the confirmed delta applied per batch |
| 813 | `CACHE_OPS_PER_BATCH` | Count | `pending_state.rs` | Number of operations per speculatively executed batch |
| 814 | `CACHE_UNORDERED_EXECUTION_TIME` | Duration | `mod.rs` | Time for unordered (read-only) rayon thread-pool execution |
| 815 | `CACHE_REBUILD_TIME` | Duration | `pending_state.rs` | Time to rebuild the accumulated cache after a confirmation or backtrack |
| 816 | `CACHE_ENQUEUE_TO_EXECUTE_LATENCY` | Duration | `mod.rs` | Time from when a preemptive update was enqueued to when execution starts |

---

## Testing

Tests for both executors live alongside their source code under `tests/` subdirectories.

### Dual-state tests (`src/single_thread_double_state/tests/`)

- `test_fixtures.rs` — Shared counter application and `make_batch` helper.
- `tests.rs` — Integration tests that spawn both workers via channels and observe emitted `AppStateMessage` checkpoints. Covers normal path, backtracking, state transfer, catch-up, and convergence between preemptive and direct execution paths.

Unit tests for the internal state machines live inline in `confirmed_requests.rs` and `preemptive_requests.rs`.

### CRUD cache tests (`src/single_threaded_crud/tests/`)

- `test_fixtures.rs` — `MapState` (HashMap-backed `CRUDState` + `MonolithicState`), `MapApp`, `NoopNode`, `make_batch`, `spawn_worker`.
- `unit_tests.rs` — Tests for `CachingPreemptiveState` in isolation (no channels, no threads). Covers preemptive accumulation, confirmation, tombstone semantics, backtracking, catch-up, and state install.
- `integration_tests.rs` — End-to-end tests via `PreemptiveExecutorHandle` + checkpoint receiver. Covers:
  - Basic preemptive → finalize
  - Multiple updates in order
  - Direct confirmed updates
  - Tombstone delete reaching confirmed state
  - CatchUp discarding speculative work
  - State transfer + resume
  - Future-seq requests being silently dropped
  - Backtrack correcting speculative state
  - Backtrack followed by continued execution
  - Convergence invariant: preemptive path and direct path must produce identical confirmed state
