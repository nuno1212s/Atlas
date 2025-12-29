# Atlas-SMR-Preemptive-Execution

<div style="text-align:center">
  <h1>⚡ Atlas SMR Preemptive Execution Framework</h1>
  <p><em>Speculative, in-memory execution layer that preemptively executes requests as soon as they arrive from the ordering protocol, with commit/discard semantics driven by consensus decisions</em></p>

  [![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
  [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
</div>

---

## 📋 Table of Contents

- [Overview](#-overview)
- [Key differences vs `Atlas-SMR-Execution`](#-key-differences-vs-atlas-smr-execution)
- [Core architecture](#-core-architecture)
- [Preemptive execution model](#-preemptive-execution-model)
- [Execution modes](#-execution-modes)
- [State management and semantics](#-state-management-and-semantics)
- [Integration with Atlas-SMR-Core](#-integration-with-atlas-smr-core)
- [Configuration and metrics](#-configuration-and-metrics)
- [Usage guide](#-usage-guide)
- [Recovery, rollback and catch-up](#-recovery-rollback-and-catch-up)
- [Design principles](#-design-principles)

## 🧭 Overview

`Atlas-SMR-Preemptive-Execution` is an execution layer designed to reduce end-to-end latency by speculatively executing requests as soon as they are produced by the ordering protocol. Unlike the standard execution module, preemptive execution begins work in memory before the ordering protocol finalizes decisions. Later, when decisions are delivered (accept/commit or fail/abort), speculative results are either committed to durable state or discarded and rolled back.

This README documents the model, integration points, expected APIs and considerations for moving from a single-threaded speculative executor to a parallel speculative executor in the future.

## 🔀 Key differences vs `Atlas-SMR-Execution`

- Preemptive execution: begin executing incoming ordered requests immediately, without waiting for final consensus decisions.
- Speculative in-memory state: speculative changes are kept isolated from the committed state until decisions arrive.
- Decision-driven commit/discard: results are committed when the ordering decides; otherwise discarded and the executor restarts from the last committed snapshot.
- Execution modes: initial implementation targets single-threaded deterministic execution; an extensible API allows eventual parallel speculative execution.
- Strong emphasis on rollback, isolation and deterministic commit semantics.

## 🏗️ Core architecture

```
Ordering Protocol (pre-decision) → Atlas-SMR-Preemptive-Execution → Application State (committed)
               ↕                                   ↕
        Decision Notifications               Commit / Discard
```

- Pre-decision requests arrive from the ordering layer and are executed speculatively in-memory.
- Each speculative execution is associated with a sequence identifier (e.g., proposal id / seq no / view).
- A decision stream reconciles speculative work: accepted proposals are committed in order; rejected ones are discarded.
- The executor exposes handles to submit preemptive requests and to notify decisions.

## ⚙️ Preemptive execution model

Principles:

- Speculative execution: execute as soon as requests are available to mask ordering latency.
- Versioned speculation: speculative writes are tagged with proposal identifiers and ordered positions.
- Commit semantics: when a decision for a proposal is accepted, apply the speculative writes (in sequence) to the committed state.
- Discard semantics: when a decision fails, drop the speculative writes and roll back to last committed snapshot, optionally re-executing dependent requests.
- Isolation: speculative changes are not visible to the committed state until they are committed.

Example conceptual types and API (illustrative):

```rust
pub enum Decision {
    Commit,
    Abort,
}

pub struct SpeculativeResult {
    proposal_id: ProposalId,
    seq_no: SeqNo,
    writes: HashMap<String, Vec<u8>>, // conceptual
}

pub enum PreemptiveExecutionRequest<O> {
    PreemptiveExecute((O, ProposalId)), // run immediately in speculative memory
    Decision(ProposalId, Decision),     // commit or discard the proposal
    ReadCommittedState(NodeId),         // read-only from committed state
    InstallState(MaybeState),           // state transfer for recovery
}
```

Notes:
- The internal representation of writes will be application/state-specific (CRUD maps, monolithic snapshots, divisible parts, etc.).
- The executor must preserve deterministic ordering when applying commits.

## 🧩 Execution modes

1. Single-threaded (initial)
   - Deterministic speculative execution with straightforward checkpoint/rollback.
   - Good for correctness-first and easier reasoning about deterministic replay.

2. Parallel / Scalable (planned)
   - Speculative parallel execution with collision detection and dependency tracking.
   - Requires application state to implement concurrent-friendly traits (e.g., `CRUDState`, `ScalableApp`).
   - Adds complexity: conflict detection, rollback of partially-applied speculative writes, and deterministic commit ordering.

The module API is designed so single-threaded and parallel executors share common message types and commit/discard semantics.

## 🗂️ State management and semantics

- Speculative layer(s): keep an in-memory layer per outstanding proposal or per execution window.
- Committed state: durable state representing the last agreed-on sequence of commits.
- Checkpoints: the executor should support creating/installing checkpoints for recovery.

Commit path:
- Receive Decision(Commit, proposal_id) for proposals in order
- Apply speculative writes associated with that proposal to the committed state
- Advance last-committed marker and free speculative memory

Discard path:
- Receive Decision(Abort, proposal_id)
- Drop speculative writes associated with that proposal
- If other speculative work depended on discarded writes, either drop/re-execute those units or trigger deterministic re-execution from last commit

API expectations:
- For parallel mode, application state should implement `CRUDState` and be `Send + Sync`.
- For single-threaded mode, applications can use `MonolithicState` or `DivisibleState` interfaces used by the rest of Atlas.

## 🔗 Integration with Atlas-SMR-Core

The executor is intended to plug into Atlas-SMR-Core similarly to `Atlas-SMR-Execution`, but with additional message types to handle preemptive execution and decision reconciliation.

Integration points:
- Work submission: a channel/handle for preemptive execution requests produced by the ordering protocol.
- Decision notifications: a decision channel that informs the executor which proposals to commit or discard.
- State transfer: the same state-install/install-checkpoint APIs used by other executors for recovery.
- Executor handle should provide methods like:
  - `submit_preemptive(request: O, proposal_id: ProposalId)`
  - `notify_decision(proposal_id: ProposalId, decision: Decision)`
  - `read_committed()` / `install_state()`

Example processing pipeline (conceptual):

```text
Ordering -> submit_preemptive -> speculative execute -> buffer speculative results
Decision -> notify_decision -> commit/discard -> update committed state
```

Concurrency model:
- For single-threaded start, channels and a dedicated thread/process perform speculative execution and decision reconciliation.
- For parallel mode, a thread-pool or task scheduler will perform speculative units and coordinate access to speculative layers.

## 📊 Configuration and metrics

Suggested configuration knobs (mirroring the Execution module where applicable):
- BUFFER_SIZES: sizes for preemptive request queues and state channels
- THREAD_POOL_THREADS: number of threads used by the parallel executor (when enabled)
- SPECULATIVE_WINDOW_SIZE: number of outstanding speculative proposals allowed

Metrics to capture:
- speculative_execution_latency: time to run speculative execution
- time_to_commit_after_decision: time between decision arrival and commit application
- speculative_discards_count: how often speculative results are discarded
- rollback_duration: time cost to rollback discarded speculation
- speculative_ops_per_second: throughput of speculative execution

Instrumentation points:
- start and end of speculative execution
- decision arrival and commit completion
- discard and rollback events

## 🚀 Usage guide

When to use preemptive execution:
- Use when ordering latency is a meaningful fraction of end-to-end latency and speculation can mask that latency.
- Start with single-threaded preemptive execution for safety and easier correctness proofs.

How to adopt:
1. Implement or reuse existing `Application<S>` and state traits (Monolithic/Divisible/CRUDState) required by the executor flavor.
2. Wire the ordering protocol to call `submit_preemptive` as proposals are formed.
3. Ensure the core calls `notify_decision` for proposals when decisions arrive.
4. Monitor speculative discard rates and rollback costs; tune `SPECULATIVE_WINDOW_SIZE` accordingly.

Best practices:
- Keep speculative writes isolated and lightweight to reduce rollback cost.
- Avoid exposing uncommitted speculative state to external reads unless the application semantics permit it.
- Provide deterministic replay paths so that re-execution after discard is straightforward.

## 🔧 Recovery, rollback and catch-up

Startup/Recovery:
- Install the last committed checkpoint before accepting new speculative work.
- Any outstanding speculative proposals from before a crash must be reconciled with decisions (commit/discard) provided by the ordering/consensus protocol.

Rollback strategy:
- Drop the speculative layer(s) that correspond to aborted proposals.
- Re-execute dependent proposals if they were affected by discarded writes.
- Optionally, keep a fast re-execution cache to speed up replay of common requests.

Catch-up:
- Use existing `CatchUp` / `InstallState` flows from Atlas-SMR-Core to synchronize committed state across replicas.
- After installing a checkpoint, the executor should replay or discard pending speculation according to the authoritative decision log.

## 🎯 Design principles

1. Latency-first: reduce perceived latency by safe speculation.
2. Determinism: commit/discard semantics must preserve deterministic state across replicas.
3. Isolation: never allow speculative writes to corrupt committed state before commit.
4. Extensibility: the API should allow plugging in single-threaded and parallel implementations without changing the core contract.
5. Observability: track speculation success/failure and rollback costs to steer configuration.