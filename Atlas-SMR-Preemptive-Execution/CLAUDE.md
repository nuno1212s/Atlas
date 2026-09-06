# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development

See the workspace-level `CLAUDE.md` (at `../CLAUDE.md`) for full build commands. This crate is built as part of the Atlas workspace:

```bash
# Build this crate specifically
cargo build -p atlas-smr-preemptive-execution

# Run tests for this crate
cargo test -p atlas-smr-preemptive-execution
cargo nextest run -p atlas-smr-preemptive-execution

# Lint
cargo clippy -p atlas-smr-preemptive-execution --no-deps -- -D warnings
```

## Crate Purpose

`atlas-smr-preemptive-execution` implements speculative (preemptive) SMR execution to hide ordering latency. The key idea: execute requests *before* consensus finalizes them, then commit or rollback based on the finalized decision.

There are two independent executor implementations in this crate, each with different trade-offs. Both share the same `PreemptiveExecutorHandle` / `PreemptiveExecutionRequest` message protocol.

---

## Architecture 1: Dual-State (`single_thread_double_state`)

The original design — lives in `src/single_thread_double_state/`. Two workers operate in parallel with two separate state copies:

```
External Input                   Thread Assignment
─────────────────────────────────────────────────────
PreemptiveUpdate ──────────────► Preemptive Worker
                                   └─ executes on preemptive_state (speculative)
                                   └─ buffers (batch, replies) in TBO queue

UpdateFinalized(seq) ──────────► Preemptive Worker
                                   └─ pops confirmed batch from queue
                                   └─ sends to Confirmed Worker

UpdateBatch (direct) ──────────► Confirmed Worker
                                   └─ executes on confirmed_state (authoritative)
                                   └─ sends replies

ExecuteUnordered ───────────────► Confirmed Worker
                                   └─ parallel reads via rayon thread pool
```

Three threads: main orchestrator, confirmed worker, preemptive worker. Plus a 4-thread rayon pool for unordered reads.

**Key types:**
- `preemptive_requests.rs` — `PreemptiveRequestPipeline` with `VSingleTBOQueue` of pending speculative updates
- `confirmed_requests.rs` — `ConfirmedRequestPipeline` for the authoritative state
- `state_management.rs` — Inter-worker message types (state snapshot request/response)

**Rollback**: Drop pending updates from the TBO queue, resync preemptive state by cloning confirmed state. Requires a full state clone on every backtrack.

**Confirmed worker re-executes every batch** — the preemptive worker sends the batch to the confirmed worker which runs it again on the authoritative state. Replies are sent from the confirmed worker.

---

## Architecture 2: CRUD Cache-Based (`single_threaded_crud`)

Added in this session. Lives in `src/single_threaded_crud/`. Single worker thread; no confirmed worker.

### Key insight
Instead of maintaining two full state copies and re-executing on confirmation, this design:
- Keeps **one confirmed state** (real state, only mutated on confirmation)
- Each preemptive update executes through a `CachingState` proxy — writes go into a per-update **delta** (RAM cache), the real state is never touched
- An **accumulated cache** = ordered merge of all pending deltas; answers reads that fall through from the current update's local delta
- On **confirmation**: apply the pre-computed delta to the real state — no re-execution
- On **backtrack**: discard pending deltas from the bad point forward, rebuild the accumulated cache — no state clone

```
External Input          Single Worker Thread
──────────────────────────────────────────────────
PreemptiveUpdate  ────► execute via CachingState proxy
                         delta → accumulated_cache → confirmed_state
                         store (batch, replies, delta) in pending queue

PreemptiveUpdateFinalized(seq) ──► pop head, apply delta to real state,
                                    rebuild accumulated_cache, send replies

UpdateBatch (direct) ──► execute directly on confirmed_state (no pending allowed)

CatchUp ──────────────► clear pending + cache, apply directly, advance seq_nos

PollStateChannel ─────► switch to StateTransfer mode (receive InstallStateMessage)

ExecuteUnordered ─────► rayon thread pool, reads from confirmed_state only
```

### File structure

```
src/single_threaded_crud/
├── mod.rs               — CachingPreemptiveWorker, spawn loop, init_handle/init_executor
├── caching_state.rs     — CachingState<'a, S>, AccumulatedCache, merge/apply/rebuild helpers
├── pending_state.rs     — CachingPreemptiveState, PendingCachedUpdate, error types
└── tests/
    ├── test_fixtures.rs — MapState, MapApp, NoopNode, make_batch, spawn_worker
    ├── unit_tests.rs    — CachingPreemptiveState unit tests (no channels, no threads)
    └── integration_tests.rs — end-to-end tests via PreemptiveExecutorHandle + checkpoint_rx
```

### Critical non-obvious constraints

**`S: Sync` required everywhere:**
`CachingState<'a, S>` contains `&'a S`. For `CachingState` to implement `CRUDState` (which is `Send`), we need `&'a S: Send`, which requires `S: Sync`. This `Sync` bound must be propagated to every struct that stores or uses `CachingState` — including `CachingPreemptiveState` and `CachingPreemptiveWorker`.

**`pub(super)` visibility is accessible to test submodules:**
Items declared `pub(super)` in `pending_state` are visible to `single_threaded_crud` (the parent) and all its descendants, including `tests::unit_tests`. No need to widen visibility to `pub(crate)` just for tests.

**`PreemptiveExecutorHandle` holds both tx and rx:**
`init_handle()` creates the channel pair and stores both ends in the handle. `init_executor()` receives a *clone* of the rx. The handle's own rx is unused — only `e_tx` (the sender) is used at runtime. This is intentional: the handle is used by callers to *send* requests, not to receive.

**Checkpoint is only emitted for `*AndGetAppstate` variants:**
Integration tests observe confirmed state via `checkpoint_rx`. This channel only receives a message when `UpdateBatchAndGetAppstate` or `PreemptiveUpdateFinalizedAndGetAppstate` is processed. All other variants are silent — design tests to use these variants as observation points.

**Replies sent on confirmation, not preemptive execution:**
Pre-computed replies are stored in the pending queue at speculative time, then sent to clients only when the update is confirmed. Clients never see speculative results.

**`update_batch` has a default impl in `Application`:**
The `Application` trait provides a default `update_batch` that calls `update` in a loop. Custom implementations are only needed for performance. `MapApp` in the test fixtures relies on this default.

**Metric ID allocation:**
- `atlas-smr-preemptive-execution` and `atlas-smr-execution` are **mutually exclusive** alternatives — their ID spaces never coexist in the same binary, so overlap is harmless.
- Dual-state metrics: IDs 800–801 (original), 804–807 (added).
- CRUD cache metrics: IDs 802–803 (original), 809–816 (added).
- Scalable CRUD metrics: IDs 817–820.
- Reorder buffer metrics: IDs 821–823, shared by **all three** executors (821 `REORDER_BUFFER_SIZE`, 822 `REORDER_STAGED_COUNT`, 823 `SPECULATION_FALLBACK_COUNT`).
- IDs 808 is reserved/unused.
- All IDs are in `src/metric.rs`. All metrics are fully wired up — no unconnected constants remain.

**Metric wiring locations:**
- `confirmed_worker/mod.rs` — 800 `CONFIRMED_WORKER_LATENCY` (recorded per `execute_and_advance` call, using `Instant` stored in `Update` enum and `PreemptiveToConfirmedMsg`)
- `confirmed_requests.rs` — 801 `CONFIRM_EXECUTION_TIME` (wraps `application.update_batch` in `execute_update`)
- `pending_state.rs` — 802, 803, 809, 810, 811, 812, 813, 815 (all CRUD cache state-machine metrics)
- `single_threaded_crud/mod.rs` — 814, 816 (unordered execution time and enqueue-to-execute latency)
- `preemptive_requests.rs` — 804, 805, 807 (dual-state preemptive execution, speculation-to-confirm latency, ops per batch)
- `preemptive_worker/mod.rs` — 806 `DS_BACKTRACK_COUNT` (in `handle_backtracking_request`)

**`speculated_at: Instant` propagation for latency metrics:**
Both `PendingCachedUpdate` (CRUD cache, in `pending_state.rs`) and `PendingPermanentUpdate` (dual-state, in `preemptive_requests.rs`) carry a `speculated_at: Instant` field set when the update enters the pending queue. This field is consumed in `handle_update_confirmed` to record the speculation-to-confirmation latency.

**`Instant` in `PreemptiveToConfirmedMsg`:**
`state_management.rs::PreemptiveToConfirmedMsg::UpdateConfirmed` and `UpdateConfirmedEmitAppState` carry an `Instant` set at send time in `comm_handles.rs`. This is propagated through the TBO queue inside the `Update` enum in `confirmed_worker/mod.rs` and consumed in `execute_and_advance` to record metric 800.

**Sequence tracking is "next expected", not "last applied":**
All three executors track the sequence number they are *waiting for* (`next_preemptive` / `next_confirmed`), not the last one applied. This is load-bearing, not cosmetic. `SeqNo::ZERO` is both the initial value and the first sequence number consensus assigns (febft is 0-indexed), so a "last applied" field cannot distinguish "nothing has run yet" from "sequence 0 has run" — under the old scheme the first batch of every run was classified as a backtrack, dropped, and its clients never got replies. The constructor's `SeqNo` argument therefore means "the next sequence expected", and `install_confirmed_state(seq, ..)` stores `seq.next()`.

Compare sequence numbers with `Ord` (`.cmp()`), not `<`/`<=`: `SeqNo`'s `PartialOrd` routes through the wrap-aware `index()`, which reports a sequence more than `PERIOD + PERIOD/2` ahead as *behind*. Using `<` there misclassifies a far-future batch as a backtrack.

**Out-of-order delivery and the reorder buffer:**
The ordering protocol decides several consensus instances concurrently and emits `DecisionRequests` at pre-prepare completion, so preemptive batches reach the executor **out of sequence** as a matter of course (measured: up to 15 positions of displacement on a 4-replica PBFT run). The decision log sends each batch exactly once over a blocking channel, so batches are never *lost* — gaps always close.

Each executor therefore holds early arrivals in a `staged: BTreeMap<SeqNo, UpdateBatch<..>>` reorder buffer instead of dropping them, and drains it forward after every successful execution. Dropping them (the old behaviour) wedged the pipeline permanently on the first out-of-order arrival: the head could never reach the dropped sequence, every later batch was rejected, and once the pending queue drained every confirmation hit `EmptyQueue` and no client got a reply again.

`MAX_STAGED_BATCHES` (4096, defined in `single_threaded_crud/pending_state.rs` and shared by all three executors) bounds the buffer. Because batches are never dropped in transit, hitting the bound means a batch was genuinely lost, which is why it logs at ERROR rather than being handled silently.

**Backtrack mechanics in the worker:**
`handle_preemptive_update` in `mod.rs` returns `PreemptiveError::Backtracking(seq, batch)` when the incoming seq is behind the current head. The worker immediately calls `self.state.backtrack(seq)` to discard the stale pending entries, then retries the same batch as a fresh preemptive update. Backtracking also evicts buffered batches at or above the backtrack point — those decisions are being re-decided, so the buffered copies are stale.

**Confirmation degrades instead of failing:**
If a confirmation arrives for a batch that was never speculated (it was still in the reorder buffer), `handle_update_confirmed` executes it directly against the confirmed state rather than erroring. Speculation already built on top of it is invalid, so those pending entries are returned to the reorder buffer — the batches survive, only the computed deltas and replies are thrown away — and speculation restarts behind the confirmed batch. The executor degrades to baseline execution rather than halting.

**For unordered reads, clone the Arc before borrowing self:**
```rust
let application = self.application.clone(); // releases borrow on self.application
let state: &S = self.state.confirmed_state();
let pool: &ThreadPool = &self.read_thread_pool;
pool.install(|| { ... application.unordered_execution(state, op) ... });
```
The borrow checker cannot split borrows of `self.application` and `self.state` unless one is cloned first.

---

## Shared Infrastructure

**`src/exec_handle.rs`** — `PreemptiveExecutionRequest<O>` enum and `PreemptiveExecutorHandle<RQ>`. Both executors share this. The handle implements:
- `TExecutionHandle` — `poll_state_channel`, `catch_up_to_quorum`, `queue_unordered`
- `TDeterministicExecutionHandle` — `queue_update`, `queue_update_and_get_appstate`
- `TPreemptiveExecutionHandle` — `queue_preemptive_update`, `queue_update_finalized`, `queue_update_finalized_and_get_appstate`

**`src/lib.rs`** — Registers both executors:
- `MonolithicPreemptiveExecutor` — wraps the dual-state design; requires only `Application<S>`
- `CRUDMonolithicPreemptiveExecutor` — wraps the CRUD cache design; requires `CRUDApplication<S>` + `CRUDState`

---

## Choosing Between the Two Executors

| Factor | Dual-State | CRUD Cache |
|---|---|---|
| State trait required | `Application<S>` only | `CRUDApplication<S>` + `CRUDState` |
| Memory overhead | 2× full state copies | 1× state + delta cache |
| Backtrack cost | Clone entire confirmed state | Discard deltas, rebuild cache |
| Confirmation cost | Full re-execution on confirmed state | Apply pre-computed delta only |
| Thread count | 3 (orchestrator + 2 workers) | 1 worker thread |
| Complexity | Higher (inter-worker channels) | Lower (single-threaded) |

Use the dual-state design when `S` does not implement `CRUDState`. Use the CRUD cache design when memory and re-execution cost matter and the application can implement the CRUD trait.

---

## Known Incomplete Areas

Several `todo!()` placeholders remain in the dual-state design (this crate is under active development on the `preemptive_execution` branch):
- `UpdateFinalizedAndGetAppstate` path in orchestrator (`mod.rs` ~line 234)
- `PollStateChannel` handling in preemptive worker (`preemptive_worker/mod.rs`)
- `ConfirmedToPreemptiveMsg` state resync in preemptive worker

The CRUD cache executor (`single_threaded_crud`) is fully implemented and tested.

**Pre-existing flaky tests:** `single_thread_double_state::tests::execution::test_state_transfer_*`
and `test_calc_*_states_converge` fail intermittently (roughly half the time) on the `todo!()`
paths above, timing out after 5s on a channel `recv`. This predates the reorder-buffer work —
verify against a stashed baseline before attributing a failure in that family to a change.
