/// Worker integration tests.
///
/// These tests exercise the preemptive worker and confirmed worker as standalone
/// concurrent components, communicating via their real channel handles. The only
/// observable "output" is:
///   - The `AppStateMessage` emitted on the `state_emission_channel` whenever the
///     confirmed worker processes an `UpdateAndGetState` variant.
///   - The confirmed-worker state snapshot returned via the
///     `PreemptiveToConfirmedMsg::RequestStateCopy` round-trip that the preemptive
///     worker uses during backtracking.
///
/// Tests are intentionally coarse-grained: they verify end-to-end channel communication
/// and correct state values rather than internal data-structure invariants (which are
/// already covered by the unit tests in `preemptive_requests` and `confirmed_requests`).
///
/// # Invariant enforced by these tests
///
/// A `ConfirmedUpdate` can only be sent when no preemptive updates are pending
/// (i.e. all previously preemptive-executed batches have been confirmed). Sending
/// a `ConfirmedUpdate(N+1)` while `PreemptiveUpdate(N)` is still unconfirmed is
/// illegal and would corrupt the pending queue's seq pointer.
use std::sync::Arc;
use std::time::Duration;

use atlas_common::channel::sync::{self, ChannelSyncRx};
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::{IncrementableUpdateBatch, UpdateBatch, UpdateInfo};
use atlas_smr_application::app::Application;
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_application::state::monolithic_state::{AppStateMessage, MonolithicState};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::reply::{ReplyNode, RequestType};
use atlas_smr_execution::repliers::FollowerReplier;
use crate::single_thread_double_state::comm_handles::initialize_shared_channels;
use crate::single_thread_double_state::confirmed_worker::comm_handles::{ConfirmedUpdateMessage, ConfirmedWorkerHandle};
use crate::single_thread_double_state::confirmed_worker::init_confirmed_worker;
use crate::single_thread_double_state::preemptive_worker::comm_handles::{PreemptiveWorkMessage, PreemptiveWorkerHandle};
use crate::single_thread_double_state::preemptive_worker::initialize_preemptive_execution;
use crate::single_thread_double_state::state_management::StateMessage;
use super::test_fixtures::{TestData, make_batch};

/// Counter state. The value is the running total of all applied requests.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Counter(u32);

impl MonolithicState for Counter {
    fn serialize_state<W>(mut w: W, s: &Self) -> atlas_common::error::Result<()>
    where
        W: std::io::Write,
    {
        w.write_all(&s.0.to_le_bytes())?;
        Ok(())
    }

    fn deserialize_state<R>(mut r: R) -> atlas_common::error::Result<Self>
    where
        R: std::io::Read,
    {
        let mut buf = [0u8; 4];
        r.read_exact(&mut buf)?;
        Ok(Counter(u32::from_le_bytes(buf)))
    }
}

/// Application: adds the request value to the counter and returns the new total.
struct CounterApp;

impl Application<Counter> for CounterApp {
    type AppData = TestData;

    fn initial_state() -> atlas_common::error::Result<Counter> {
        Ok(Counter(0))
    }

    fn unordered_execution(&self, state: &Counter, _: u32) -> u32 {
        state.0
    }

    fn update(&self, state: &mut Counter, req: u32) -> u32 {
        state.0 += req;
        state.0
    }
}

/// No-op network node — workers using `FollowerReplier` never actually call it for
/// ordered requests, so a unit struct is sufficient.
struct NoopNode;

impl ReplyNode<SMRReply<TestData>> for NoopNode {
    fn send(
        &self,
        _rt: RequestType,
        _reply: SMRReply<TestData>,
        _target: NodeId,
        _flush: bool,
    ) -> atlas_common::error::Result<()> {
        Ok(())
    }

    fn send_signed(
        &self,
        _rt: RequestType,
        _reply: SMRReply<TestData>,
        _target: NodeId,
        _flush: bool,
    ) -> atlas_common::error::Result<()> {
        Ok(())
    }

    fn broadcast(
        &self,
        _rt: RequestType,
        _reply: SMRReply<TestData>,
        _targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        Ok(())
    }

    fn broadcast_signed(
        &self,
        _rt: RequestType,
        _reply: SMRReply<TestData>,
        _targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        Ok(())
    }
}


/// Timeout used for all blocking channel reads to avoid hanging tests.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn both workers and return their handles together with the `state_emission`
/// receiver that the test can use to observe confirmed-state changes.
#[allow(clippy::type_complexity)]
fn spawn_workers() -> (
    PreemptiveWorkerHandle<u32, Counter>,
    ConfirmedWorkerHandle<u32, Counter>,
    ChannelSyncRx<AppStateMessage<Counter>>,
) {
    let app = Arc::new(CounterApp);
    let node = Arc::new(NoopNode);

    let (state_emission_tx, state_emission_rx) =
        sync::new_bounded_sync(64, Some("test_state_emission"));

    let (confirmed_shared, preemptive_shared) = initialize_shared_channels(state_emission_tx);

    let confirmed_handle = init_confirmed_worker::<CounterApp, Counter, NoopNode, FollowerReplier>(
        (SeqNo::ZERO, Counter(0)),
        app.clone(),
        node.clone(),
        confirmed_shared,
    );

    let preemptive_handle =
        initialize_preemptive_execution::<CounterApp, Counter, NoopNode, FollowerReplier>(
            (SeqNo::ZERO, Counter(0)),
            app.clone(),
            node.clone(),
            preemptive_shared,
        );

    (preemptive_handle, confirmed_handle, state_emission_rx)
}

// ---------------------------------------------------------------------------
// Preemptive worker → Confirmed worker: normal path
// ---------------------------------------------------------------------------

/// Preemptive update followed by `PreemptiveUpdateConfirmedAndGetAppState` causes
/// the confirmed worker to execute the batch and emit an `AppStateMessage`.
#[test]
fn test_preemptive_then_confirmed_emits_correct_state() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(
            1,
            &[10],
        )))
        .unwrap();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(1u32)))
        .unwrap();

    let msg = state_rx
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out waiting for AppStateMessage");
    assert_eq!(msg.seq(), SeqNo::from(1u32));
    assert_eq!(msg.state().0, 10u32);
}

/// Two sequential preemptive updates, confirmed in order, produce the correct
/// cumulative state in the confirmed worker.
#[test]
fn test_multiple_preemptive_updates_confirmed_in_order() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(1, &[3])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(2, &[7])))
        .unwrap();

    // Confirm seq 1 without requesting state
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(1u32),
        ))
        .unwrap();

    // Confirm seq 2 and request state emission
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(2u32)))
        .unwrap();

    let msg = state_rx
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out waiting for AppStateMessage");
    assert_eq!(msg.seq(), SeqNo::from(2u32));
    assert_eq!(msg.state().0, 10u32); // 3 + 7
}

// ---------------------------------------------------------------------------
// ConfirmedUpdate path: directly-finalized batches through the preemptive worker
// ---------------------------------------------------------------------------

/// A single `ConfirmedUpdate` (no prior speculative execution) routes through the
/// preemptive worker, which executes it and forwards to the confirmed worker.
/// The confirmed worker emits the correct state via `ConfirmedUpdateAndGetAppstate`.
#[test]
fn test_confirmed_update_emits_correct_state() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_batch(1, &[42]),
        ))
        .unwrap();

    let msg = state_rx
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out waiting for AppStateMessage");
    assert_eq!(msg.seq(), SeqNo::from(1u32));
    assert_eq!(msg.state().0, 42u32);
}

/// Multiple `ConfirmedUpdate` batches in sequence (no preemptive pending) accumulate
/// correctly in both preemptive and confirmed state.
#[test]
fn test_multiple_confirmed_updates_accumulate() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    // Both batches have no prior pending preemptive work — legal.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdate(make_batch(1, &[5])))
        .unwrap();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_batch(2, &[10]),
        ))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(2u32));
    assert_eq!(msg.state().0, 15u32); // 5 + 10
}

// ---------------------------------------------------------------------------
// Mixed preemptive and confirmed updates
// ---------------------------------------------------------------------------

/// All pending preemptive work confirmed, then a directly-finalized batch applied.
/// This is the canonical legal interleaving: confirmed updates may only arrive once
/// the preemptive queue is empty.
#[test]
fn test_confirmed_update_after_all_preemptive_confirmed() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    // Preemptive seq 1, then confirmed — queue is now empty.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(
            1,
            &[10],
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(1u32),
        ))
        .unwrap();

    // ConfirmedUpdate at seq 2: legal because no preemptive work is pending.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_batch(2, &[5]),
        ))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(2u32));
    assert_eq!(msg.state().0, 15u32); // 10 + 5
}

/// Two preemptive batches confirmed before a directly-finalized batch.
/// The preemptive queue must be fully drained (both seq 1 and seq 2 confirmed)
/// before the ConfirmedUpdate at seq 3 is sent.
#[test]
fn test_confirmed_update_after_multiple_preemptive_confirmed() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(1, &[3])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(2, &[7])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(1u32),
        ))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(2u32),
        ))
        .unwrap();

    // Queue is empty; confirmed update at seq 3 is legal.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_batch(3, &[20]),
        ))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(3u32));
    assert_eq!(msg.state().0, 30u32); // 3 + 7 + 20
}

/// Alternating blocks: preemptive→confirm, then confirmed, then preemptive→confirm.
/// Verifies that the seq-no tracking across mode switches stays coherent.
#[test]
fn test_interleaved_preemptive_and_confirmed_blocks() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    // Block 1: preemptive seq 1 speculated and confirmed.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(
            1,
            &[10],
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(1u32),
        ))
        .unwrap();

    // Block 2: directly-finalized seq 2 (queue is empty — legal).
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdate(make_batch(2, &[5])))
        .unwrap();

    // Block 3: preemptive seq 3 speculated and confirmed; also request state emission.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(
            3,
            &[20],
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(3u32)))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(3u32));
    assert_eq!(msg.state().0, 35u32); // 10 + 5 + 20
}

/// Three alternating blocks to stress the seq-no handoff across multiple switches.
#[test]
fn test_multiple_alternating_preemptive_and_confirmed_blocks() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    // Block 1: preemptive seq 1
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(1, &[1])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(1u32),
        ))
        .unwrap();

    // Block 2: confirmed seq 2 and 3
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdate(make_batch(2, &[2])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdate(make_batch(3, &[3])))
        .unwrap();

    // Block 3: preemptive seq 4 and 5
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(4, &[4])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(5, &[5])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(4u32),
        ))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(5u32),
        ))
        .unwrap();

    // Block 4: confirmed seq 6 (request state emission)
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_batch(6, &[6]),
        ))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(6u32));
    assert_eq!(msg.state().0, 21u32); // 1+2+3+4+5+6
}

// ---------------------------------------------------------------------------
// State transfer
// ---------------------------------------------------------------------------

/// State transfer to both workers resets them; a directly-confirmed batch applied
/// after the transfer builds on the installed state.
#[test]
fn test_state_transfer_then_confirmed_update() {
    let (preemptive, confirmed, state_rx) = spawn_workers();

    // State transfer to both workers — the orchestrator always does this in production.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PollStateChannel)
        .unwrap();
    preemptive
        .preemptive_state_handle()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(5u32),
            Counter(100),
        ))
        .unwrap();

    confirmed
        .update_messages()
        .send(ConfirmedUpdateMessage::StateTransferAvailable)
        .unwrap();
    confirmed
        .state_message_tx()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(5u32),
            Counter(100),
        ))
        .unwrap();

    // No pending preemptive work after state transfer; confirmed update at seq 6 is legal.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_batch(6, &[10]),
        ))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(6u32));
    assert_eq!(msg.state().0, 110u32); // installed 100, then +10
}

/// State transfer on both workers resets them; new preemptive updates
/// must start from the installed seq.
#[test]
fn test_state_transfer_preemptive_worker_then_confirm() {
    let (preemptive, confirmed, state_rx) = spawn_workers();

    // Some speculative work before the transfer
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(
            1,
            &[99],
        )))
        .unwrap();

    // Trigger PollStateChannel so the preemptive worker enters StateTransfer mode.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PollStateChannel)
        .unwrap();

    // Deliver the new state (seq 5, value 50) to the preemptive worker …
    preemptive
        .preemptive_state_handle()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(5u32),
            Counter(50),
        ))
        .unwrap();

    // … and to the confirmed worker so its queue is also reset to seq 6.
    confirmed
        .update_messages()
        .send(ConfirmedUpdateMessage::StateTransferAvailable)
        .unwrap();
    confirmed
        .state_message_tx()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(5u32),
            Counter(50),
        ))
        .unwrap();

    // After the state transfer the workers are back to Normal; continue from seq 6.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(6, &[5])))
        .unwrap();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(6u32)))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(6u32));
    assert_eq!(msg.state().0, 55u32); // 50 + 5
}

// ---------------------------------------------------------------------------
// State transfer + CatchUp
// ---------------------------------------------------------------------------

/// After a state transfer the catch-up path advances both workers and resumes
/// normal preemptive execution from the new HEAD.
#[test]
fn test_state_transfer_and_catchup_then_resume() {
    let (preemptive, confirmed, state_rx) = spawn_workers();

    // ── State transfer to seq 3 (both workers) ──────────────────────────────
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PollStateChannel)
        .unwrap();
    preemptive
        .preemptive_state_handle()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(3u32),
            Counter(30),
        ))
        .unwrap();

    confirmed
        .update_messages()
        .send(ConfirmedUpdateMessage::StateTransferAvailable)
        .unwrap();
    confirmed
        .state_message_tx()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(3u32),
            Counter(30),
        ))
        .unwrap();

    // ── CatchUp: seq 4 already decided, deliver to both workers ─────────────
    confirmed
        .update_messages()
        .send(ConfirmedUpdateMessage::CatchUp(MaybeVec::from_one(
            make_batch(4, &[10]),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::CatchUp(MaybeVec::from_one(
            make_batch(4, &[10]),
        )))
        .unwrap();

    // ── Resume: preemptive seq 5, then confirm ───────────────────────────────
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(5, &[7])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(5u32)))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(5u32));
    // Confirmed worker: base 30, catch-up +10 → 40, confirm seq 5 +7 → 47
    assert_eq!(msg.state().0, 47u32);
}

// ---------------------------------------------------------------------------
// Backtrack
// ---------------------------------------------------------------------------

/// When a preemptive update arrives at a seq behind the current speculative HEAD
/// the preemptive worker requests the confirmed state, performs a backtrack, and
/// re-executes the update. Subsequent confirmations must still work correctly.
#[test]
fn test_backtrack_reexecutes_on_fresh_confirmed_state() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    // Speculate seq 1 (+10) and seq 2 (+20)
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(
            1,
            &[10],
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(
            2,
            &[20],
        )))
        .unwrap();

    // Deliver a "stale" update at seq 1 (behind HEAD=2) — triggers backtrack.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(1, &[5])))
        .unwrap();

    // Confirm seq 1 and request state
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(1u32)))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(1u32));
    assert_eq!(msg.state().0, 5u32);
}

// ---------------------------------------------------------------------------
// Invalid scenarios
// ---------------------------------------------------------------------------

/// An update whose seq is more than 1 ahead of the current preemptive HEAD is
/// silently dropped (FutureRequest error is logged, not propagated). The worker
/// continues to function normally afterwards.
#[test]
fn test_update_ahead_of_head_is_dropped_and_worker_continues() {
    let (preemptive, _confirmed, state_rx) = spawn_workers();

    // HEAD = 0; jump to seq 3 — skips seq 1 and 2, should be dropped
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(
            3,
            &[99],
        )))
        .unwrap();

    // Now send the correct seq 1 — worker must still accept it
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_batch(1, &[7])))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(1u32)))
        .unwrap();

    let msg = state_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.state().0, 7u32); // only the valid seq-1 batch was applied
}

// ===========================================================================
// Calculator mock application — preemptive state vs confirmed state convergence
// ===========================================================================
//
// Each test runs the same sequence of operations through TWO independent
// worker pairs:
//
//   Path A — preemptive speculation + confirmation
//             Batches flow PreemptiveUpdate → PreemptiveUpdateConfirmed.
//
//   Path B — directly-confirmed execution (no speculation)
//             Batches are sent as ConfirmedUpdate to the preemptive worker,
//             which forwards to the confirmed worker without prior speculation.
//
// If both paths produce the same final confirmed state, the preemptive worker
// produced the correct batches and the confirmed worker applied them correctly.
// This is the convergence invariant of the two-worker architecture.

/// Operations supported by the calculator application.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum CalcOp {
    Add(i64),
    Sub(i64),
    Mul(i64),
}

/// Calculator `ApplicationData` — request is a `CalcOp`, reply is the new value.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CalcData;

impl ApplicationData for CalcData {
    type Request = CalcOp;
    type Reply = i64;

    fn serialize_request<W>(_: W, _: &CalcOp) -> atlas_common::error::Result<()>
    where
        W: std::io::Write,
    {
        Ok(())
    }

    fn deserialize_request<R>(_: R) -> atlas_common::error::Result<CalcOp>
    where
        R: std::io::Read,
    {
        Ok(CalcOp::Add(0))
    }

    fn serialize_reply<W>(_: W, _: &i64) -> atlas_common::error::Result<()>
    where
        W: std::io::Write,
    {
        Ok(())
    }

    fn deserialize_reply<R>(_: R) -> atlas_common::error::Result<i64>
    where
        R: std::io::Read,
    {
        Ok(0)
    }
}

/// Calculator state: a single signed integer.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CalcState(i64);

impl MonolithicState for CalcState {
    fn serialize_state<W>(mut w: W, s: &Self) -> atlas_common::error::Result<()>
    where
        W: std::io::Write,
    {
        w.write_all(&s.0.to_le_bytes())?;
        Ok(())
    }

    fn deserialize_state<R>(mut r: R) -> atlas_common::error::Result<Self>
    where
        R: std::io::Read,
    {
        let mut buf = [0u8; 8];
        r.read_exact(&mut buf)?;
        Ok(CalcState(i64::from_le_bytes(buf)))
    }
}

/// Application: applies a `CalcOp` to the calculator state.
struct CalcApp;

impl Application<CalcState> for CalcApp {
    type AppData = CalcData;

    fn initial_state() -> atlas_common::error::Result<CalcState> {
        Ok(CalcState(0))
    }

    fn unordered_execution(&self, state: &CalcState, _: CalcOp) -> i64 {
        state.0
    }

    fn update(&self, state: &mut CalcState, op: CalcOp) -> i64 {
        match op {
            CalcOp::Add(n) => state.0 += n,
            CalcOp::Sub(n) => state.0 -= n,
            CalcOp::Mul(n) => state.0 *= n,
        }
        state.0
    }
}

/// No-op network node for calculator tests.
struct CalcNoopNode;

impl ReplyNode<SMRReply<CalcData>> for CalcNoopNode {
    fn send(
        &self,
        _rt: RequestType,
        _reply: SMRReply<CalcData>,
        _target: NodeId,
        _flush: bool,
    ) -> atlas_common::error::Result<()> {
        Ok(())
    }

    fn send_signed(
        &self,
        _rt: RequestType,
        _reply: SMRReply<CalcData>,
        _target: NodeId,
        _flush: bool,
    ) -> atlas_common::error::Result<()> {
        Ok(())
    }

    fn broadcast(
        &self,
        _rt: RequestType,
        _reply: SMRReply<CalcData>,
        _targets: impl Iterator<Item = NodeId>,
    ) -> Result<(), Vec<NodeId>> {
        Ok(())
    }

    fn broadcast_signed(
        &self,
        _rt: RequestType,
        _reply: SMRReply<CalcData>,
        _targets: impl Iterator<Item = NodeId>,
    ) -> Result<(), Vec<NodeId>> {
        Ok(())
    }
}

/// Build a single-op `UpdateBatch<CalcOp>` at the given seq.
fn make_calc_batch(seq: u32, op: CalcOp) -> UpdateBatch<CalcOp> {
    let mut batch = UpdateBatch::new(SeqNo::from(seq));
    batch.add(
        UpdateInfo::new_session_based(NodeId::from(0u32), SeqNo::ZERO, SeqNo::from(seq)),
        op,
    );
    batch
}

/// Spawn both calculator workers and return handles + the state-emission receiver.
#[allow(clippy::type_complexity)]
fn spawn_calc_workers() -> (
    PreemptiveWorkerHandle<CalcOp, CalcState>,
    ConfirmedWorkerHandle<CalcOp, CalcState>,
    ChannelSyncRx<AppStateMessage<CalcState>>,
) {
    let app = Arc::new(CalcApp);
    let node = Arc::new(CalcNoopNode);

    let (state_emission_tx, state_emission_rx) =
        sync::new_bounded_sync(64, Some("calc_test_state_emission"));

    let (confirmed_shared, preemptive_shared) = initialize_shared_channels(state_emission_tx);

    let confirmed_handle = init_confirmed_worker::<CalcApp, CalcState, CalcNoopNode, FollowerReplier>(
        (SeqNo::ZERO, CalcState(0)),
        app.clone(),
        node.clone(),
        confirmed_shared,
    );

    let preemptive_handle =
        initialize_preemptive_execution::<CalcApp, CalcState, CalcNoopNode, FollowerReplier>(
            (SeqNo::ZERO, CalcState(0)),
            app.clone(),
            node.clone(),
            preemptive_shared,
        );

    (preemptive_handle, confirmed_handle, state_emission_rx)
}

/// Assert that the preemptive-path confirmed state and the direct-path
/// confirmed state agree, and that both equal `expected`.
fn assert_states_converge(
    preemptive_path: &AppStateMessage<CalcState>,
    direct_path: &AppStateMessage<CalcState>,
    expected: i64,
) {
    let preemptive_val = preemptive_path.state().0;
    let direct_val = direct_path.state().0;
    assert_eq!(
        preemptive_val, expected,
        "preemptive path: got {preemptive_val}, expected {expected}"
    );
    assert_eq!(
        direct_val, expected,
        "direct path: got {direct_val}, expected {expected}"
    );
    assert_eq!(
        preemptive_val, direct_val,
        "preemptive path ({preemptive_val}) != direct path ({direct_val})"
    );
}

// ---------------------------------------------------------------------------
// Calculator convergence tests
// ---------------------------------------------------------------------------

/// Sequential Add → Mul → Sub operations all confirmed in order.
///
/// Path A: speculate seq 1–3, confirm in order.
/// Path B: send same ops as ConfirmedUpdate (no speculation).
///
/// Computation: 0 + 10 = 10 → 10 × 3 = 30 → 30 − 5 = 25
#[test]
fn test_calc_sequential_ops_states_converge() {
    // ── Path A: preemptive speculation + confirmation ──────────────────────
    let (preemptive, _confirmed, state_rx_a) = spawn_calc_workers();

    for (seq, op) in [
        (1, CalcOp::Add(10)),
        (2, CalcOp::Mul(3)),
        (3, CalcOp::Sub(5)),
    ] {
        preemptive
            .preemptive_exec_handle()
            .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
                seq, op,
            )))
            .unwrap();
    }
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(1u32),
        ))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(2u32),
        ))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(3u32)))
        .unwrap();
    let result_a = state_rx_a
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path A)");

    // ── Path B: directly-confirmed (no speculation) ────────────────────────
    let (preemptive_b, _confirmed_b, state_rx_b) = spawn_calc_workers();

    for (seq, op) in [(1, CalcOp::Add(10)), (2, CalcOp::Mul(3))] {
        preemptive_b
            .preemptive_exec_handle()
            .send(PreemptiveWorkMessage::ConfirmedUpdate(make_calc_batch(
                seq, op,
            )))
            .unwrap();
    }
    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_calc_batch(3, CalcOp::Sub(5)),
        ))
        .unwrap();
    let result_b = state_rx_b
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path B)");

    assert_states_converge(&result_a, &result_b, 25);
}

/// Mixed: some batches preemptively executed and confirmed, then one directly confirmed.
///
/// Path A: speculate seq 1 (+10), confirm, speculate seq 2 (×3), confirm, then
///         directly-confirmed seq 3 (−5).
/// Path B: all three as ConfirmedUpdate (no speculation).
///
/// Computation: (0+10)×3 − 5 = 25
#[test]
fn test_calc_mixed_preemptive_and_confirmed_converge() {
    // ── Path A ────────────────────────────────────────────────────────────
    let (preemptive, _confirmed, state_rx_a) = spawn_calc_workers();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
            1,
            CalcOp::Add(10),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
            2,
            CalcOp::Mul(3),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(1u32),
        ))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(2u32),
        ))
        .unwrap();

    // Queue empty — ConfirmedUpdate at seq 3 is legal.
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_calc_batch(3, CalcOp::Sub(5)),
        ))
        .unwrap();
    let result_a = state_rx_a
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path A)");

    // ── Path B: all directly-confirmed ────────────────────────────────────
    let (preemptive_b, _confirmed_b, state_rx_b) = spawn_calc_workers();

    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdate(make_calc_batch(
            1,
            CalcOp::Add(10),
        )))
        .unwrap();
    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdate(make_calc_batch(
            2,
            CalcOp::Mul(3),
        )))
        .unwrap();
    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_calc_batch(3, CalcOp::Sub(5)),
        ))
        .unwrap();
    let result_b = state_rx_b
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path B)");

    assert_states_converge(&result_a, &result_b, 25);
}

/// A wrong speculation at seq 1 and 2, then a conflicting batch at seq 1
/// triggers a backtrack.  After re-execution both paths must agree.
///
/// Path A: speculate Add(100)+Mul(2), receive Add(7) at seq 1 (backtrack), confirm.
/// Path B: directly confirmed Add(7) at seq 1.
/// Expected: both = 7
#[test]
fn test_calc_backtrack_states_converge() {
    // ── Path A: backtrack ──────────────────────────────────────────────────
    let (preemptive, _confirmed, state_rx_a) = spawn_calc_workers();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
            1,
            CalcOp::Add(100),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
            2,
            CalcOp::Mul(2),
        )))
        .unwrap();
    // Conflicting seq-1 update triggers backtrack
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
            1,
            CalcOp::Add(7),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(1u32)))
        .unwrap();
    let result_a = state_rx_a
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path A)");

    // ── Path B: directly confirmed ────────────────────────────────────────
    let (preemptive_b, _confirmed_b, state_rx_b) = spawn_calc_workers();

    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_calc_batch(1, CalcOp::Add(7)),
        ))
        .unwrap();
    let result_b = state_rx_b
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path B)");

    assert_states_converge(&result_a, &result_b, 7);
}

/// After a state transfer both workers resume from the installed state.
/// Preemptive speculation and direct confirmed execution must reach the same
/// final state.
///
/// Install state = 50 at seq 3. Then: Mul(2) at seq 4 → 100, Sub(20) at seq 5 → 80.
#[test]
fn test_calc_state_transfer_states_converge() {
    // ── Path A: preemptive path ────────────────────────────────────────────
    let (preemptive, confirmed_a, state_rx_a) = spawn_calc_workers();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PollStateChannel)
        .unwrap();
    preemptive
        .preemptive_state_handle()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(3u32),
            CalcState(50),
        ))
        .unwrap();
    confirmed_a
        .update_messages()
        .send(ConfirmedUpdateMessage::StateTransferAvailable)
        .unwrap();
    confirmed_a
        .state_message_tx()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(3u32),
            CalcState(50),
        ))
        .unwrap();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
            4,
            CalcOp::Mul(2),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
            5,
            CalcOp::Sub(20),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmed(
            SeqNo::from(4u32),
        ))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(5u32)))
        .unwrap();
    let result_a = state_rx_a
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path A)");

    // ── Path B: state transfer + directly-confirmed ────────────────────────
    let (preemptive_b, confirmed_b, state_rx_b) = spawn_calc_workers();

    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PollStateChannel)
        .unwrap();
    preemptive_b
        .preemptive_state_handle()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(3u32),
            CalcState(50),
        ))
        .unwrap();
    confirmed_b
        .update_messages()
        .send(ConfirmedUpdateMessage::StateTransferAvailable)
        .unwrap();
    confirmed_b
        .state_message_tx()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(3u32),
            CalcState(50),
        ))
        .unwrap();

    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdate(make_calc_batch(
            4,
            CalcOp::Mul(2),
        )))
        .unwrap();
    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_calc_batch(5, CalcOp::Sub(20)),
        ))
        .unwrap();
    let result_b = state_rx_b
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path B)");

    // 50 × 2 = 100 ; 100 − 20 = 80
    assert_states_converge(&result_a, &result_b, 80);
}

/// State transfer then catch-up then resumed execution.
///
/// Install state = 10 at seq 2. Catch-up: Add(5) at seq 3 → 15.
/// Resume: Mul(4) at seq 4 → 60.
#[test]
fn test_calc_catchup_then_resume_states_converge() {
    // ── Path A: preemptive path ────────────────────────────────────────────
    let (preemptive, confirmed_a, state_rx_a) = spawn_calc_workers();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PollStateChannel)
        .unwrap();
    preemptive
        .preemptive_state_handle()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(2u32),
            CalcState(10),
        ))
        .unwrap();
    confirmed_a
        .update_messages()
        .send(ConfirmedUpdateMessage::StateTransferAvailable)
        .unwrap();
    confirmed_a
        .state_message_tx()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(2u32),
            CalcState(10),
        ))
        .unwrap();

    confirmed_a
        .update_messages()
        .send(ConfirmedUpdateMessage::CatchUp(MaybeVec::from_one(
            make_calc_batch(3, CalcOp::Add(5)),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::CatchUp(MaybeVec::from_one(
            make_calc_batch(3, CalcOp::Add(5)),
        )))
        .unwrap();

    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdate(make_calc_batch(
            4,
            CalcOp::Mul(4),
        )))
        .unwrap();
    preemptive
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(SeqNo::from(4u32)))
        .unwrap();
    let result_a = state_rx_a
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path A)");

    // ── Path B: state transfer + catchup + directly-confirmed ─────────────
    let (preemptive_b, confirmed_b, state_rx_b) = spawn_calc_workers();

    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::PollStateChannel)
        .unwrap();
    preemptive_b
        .preemptive_state_handle()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(2u32),
            CalcState(10),
        ))
        .unwrap();
    confirmed_b
        .update_messages()
        .send(ConfirmedUpdateMessage::StateTransferAvailable)
        .unwrap();
    confirmed_b
        .state_message_tx()
        .send(StateMessage::ConfirmedStateReceived(
            SeqNo::from(2u32),
            CalcState(10),
        ))
        .unwrap();

    confirmed_b
        .update_messages()
        .send(ConfirmedUpdateMessage::CatchUp(MaybeVec::from_one(
            make_calc_batch(3, CalcOp::Add(5)),
        )))
        .unwrap();
    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::CatchUp(MaybeVec::from_one(
            make_calc_batch(3, CalcOp::Add(5)),
        )))
        .unwrap();

    // Queue is empty after catch-up; directly-confirmed seq 4 is legal.
    preemptive_b
        .preemptive_exec_handle()
        .send(PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(
            make_calc_batch(4, CalcOp::Mul(4)),
        ))
        .unwrap();
    let result_b = state_rx_b
        .recv_timeout(RECV_TIMEOUT)
        .expect("timed out (path B)");

    // (10 + 5) × 4 = 60
    assert_states_converge(&result_a, &result_b, 60);
}
