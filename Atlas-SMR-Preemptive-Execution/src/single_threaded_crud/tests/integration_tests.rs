//! End-to-end integration tests for the CRUD preemptive executor.
//!
//! Each test drives the worker through its real channel API
//! (`PreemptiveExecutorHandle`) and observes confirmed-state changes via the
//! checkpoint receiver the worker emits for `*AndGetAppstate` variants.
//!
//! # Convergence invariant
//!
//! Several tests run the same sequence of operations through two independent
//! worker instances:
//!
//! * **Path A** — preemptive speculation + finalization.
//! * **Path B** — directly-confirmed execution (no speculation).
//!
//! If both paths produce the same final confirmed state the executor is correct.

use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::SeqNo;
use atlas_smr_application::TExecutionHandle;
use atlas_smr_application::deterministic_execution::TDeterministicExecutionHandle;
use atlas_smr_application::preemptive_execution::TPreemptiveExecutionHandle;
use atlas_smr_application::state::monolithic_state::InstallStateMessage;
use atlas_smr_execution::crud_states::CRUDState;

use super::test_fixtures::{MapState, RECV_TIMEOUT, make_batch, spawn_worker};

// ---------------------------------------------------------------------------
// Basic preemptive + finalize
// ---------------------------------------------------------------------------

/// Single preemptive write finalized — checkpoint shows the value in confirmed state.
#[test]
fn test_basic_preemptive_then_finalize() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(0, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]))
        .unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(0u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(0u32));
    assert_eq!(msg.state().read("default", b"k1"), Some(b"v1".to_vec()));
}

/// Multiple preemptive updates confirmed in order — all writes visible in final state.
#[test]
fn test_multiple_preemptive_confirmed_in_order() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(0, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]))
        .unwrap();
    handle
        .queue_preemptive_update(make_batch(1, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]))
        .unwrap();
    handle
        .queue_preemptive_update(make_batch(2, &[(b"k3".to_vec(), Some(b"v3".to_vec()))]))
        .unwrap();

    handle.queue_update_finalized(SeqNo::from(0u32)).unwrap();
    handle.queue_update_finalized(SeqNo::from(1u32)).unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(2u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(2u32));
    let state = msg.state();
    assert_eq!(state.read("default", b"k1"), Some(b"v1".to_vec()));
    assert_eq!(state.read("default", b"k2"), Some(b"v2".to_vec()));
    assert_eq!(state.read("default", b"k3"), Some(b"v3".to_vec()));
}

// ---------------------------------------------------------------------------
// Direct confirmed update (no prior speculation)
// ---------------------------------------------------------------------------

/// `UpdateBatch` (directly finalized, no prior speculation) applies correctly.
#[test]
fn test_direct_confirmed_update() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_update_and_get_appstate(make_batch(1, &[(b"k1".to_vec(), Some(b"direct".to_vec()))]))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(1u32));
    assert_eq!(msg.state().read("default", b"k1"), Some(b"direct".to_vec()));
}

/// Multiple direct confirmed updates accumulate correctly.
#[test]
fn test_multiple_direct_confirmed_updates() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_update(make_batch(1, &[(b"k1".to_vec(), Some(b"a".to_vec()))]))
        .unwrap();
    handle
        .queue_update_and_get_appstate(make_batch(2, &[(b"k2".to_vec(), Some(b"b".to_vec()))]))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(2u32));
    let state = msg.state();
    assert_eq!(state.read("default", b"k1"), Some(b"a".to_vec()));
    assert_eq!(state.read("default", b"k2"), Some(b"b".to_vec()));
}

// ---------------------------------------------------------------------------
// Tombstone: delete through the cache reaches confirmed state
// ---------------------------------------------------------------------------

/// A preemptive delete is flushed as a tombstone on confirmation and removes
/// the key from the real state.
#[test]
fn test_tombstone_delete_confirmed() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    // Establish k1 via a direct confirmed update.
    handle
        .queue_update(make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]))
        .unwrap();
    // Preemptive seq 2: delete k1.
    handle
        .queue_preemptive_update(make_batch(2, &[(b"k1".to_vec(), None)]))
        .unwrap();
    // Finalize seq 2 and emit checkpoint.
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(2u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(2u32));
    assert!(msg.state().read("default", b"k1").is_none());
}

// ---------------------------------------------------------------------------
// CatchUp
// ---------------------------------------------------------------------------

/// `CatchUp` discards speculative work and directly applies the given batches.
#[test]
fn test_catch_up_discards_pending_and_applies_batches() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    // Speculate something that will be thrown away.
    handle
        .queue_preemptive_update(make_batch(
            1,
            &[(b"speculative".to_vec(), Some(b"gone".to_vec()))],
        ))
        .unwrap();

    // CatchUp replaces the speculative batch with authoritative ones.
    handle
        .catch_up_to_quorum(MaybeVec::from_many(vec![
            make_batch(1, &[(b"k1".to_vec(), Some(b"caught".to_vec()))]),
            make_batch(2, &[(b"k2".to_vec(), Some(b"up".to_vec()))]),
        ]))
        .unwrap();

    // Queue is now empty — direct update at seq 3 is legal.
    handle
        .queue_update_and_get_appstate(make_batch(3, &[(b"k3".to_vec(), Some(b"after".to_vec()))]))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(3u32));
    let state = msg.state();
    assert_eq!(state.read("default", b"k1"), Some(b"caught".to_vec()));
    assert_eq!(state.read("default", b"k2"), Some(b"up".to_vec()));
    assert_eq!(state.read("default", b"k3"), Some(b"after".to_vec()));
    assert!(state.read("default", b"speculative").is_none());
}

// ---------------------------------------------------------------------------
// State transfer
// ---------------------------------------------------------------------------

/// `PollStateChannel` + installing a new state resets the worker; subsequent
/// updates build on the installed state.
#[test]
fn test_state_transfer_then_resume() {
    let (handle, state_tx, checkpoint_rx) = spawn_worker();

    // Some speculative work before the transfer.
    handle
        .queue_preemptive_update(make_batch(1, &[(b"old".to_vec(), Some(b"data".to_vec()))]))
        .unwrap();

    // Signal the worker to switch to StateTransfer mode.
    handle.poll_state_channel().unwrap();

    // Install a new state at seq 5.
    let mut installed = MapState::default();
    installed.update("default", b"base", b"installed");
    state_tx
        .send(InstallStateMessage::new(SeqNo::from(5u32), installed))
        .expect("state install send failed");

    // Queue is empty after state transfer — direct update at seq 6 is legal.
    handle
        .queue_update_and_get_appstate(make_batch(6, &[(b"new".to_vec(), Some(b"write".to_vec()))]))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(6u32));
    let state = msg.state();
    assert_eq!(state.read("default", b"base"), Some(b"installed".to_vec()));
    assert_eq!(state.read("default", b"new"), Some(b"write".to_vec()));
    assert!(state.read("default", b"old").is_none());
}

// ---------------------------------------------------------------------------
// Out-of-order arrival: buffered, then replayed
// ---------------------------------------------------------------------------

/// An update whose seq is ahead of the current head is held in the reorder buffer, not
/// dropped. Once the batches in front of it arrive it is replayed, so every decision is
/// still executed and every client still gets a reply.
///
/// Regression: the executor used to drop these. Because the ordering protocol decides
/// several instances concurrently, batches routinely arrive out of order, and a single
/// dropped batch wedged the speculative pipeline for the rest of the run.
#[test]
fn test_out_of_order_update_is_buffered_and_replayed() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    // Head is at 0; seq 2 arrives first and must be held.
    handle
        .queue_preemptive_update(make_batch(2, &[(b"k2".to_vec(), Some(b"third".to_vec()))]))
        .unwrap();
    // Seq 1 also arrives early.
    handle
        .queue_preemptive_update(make_batch(1, &[(b"k1".to_vec(), Some(b"second".to_vec()))]))
        .unwrap();
    // Seq 0 closes the gap and releases both behind it.
    handle
        .queue_preemptive_update(make_batch(0, &[(b"k0".to_vec(), Some(b"first".to_vec()))]))
        .unwrap();

    handle.queue_update_finalized(SeqNo::from(0u32)).unwrap();
    handle.queue_update_finalized(SeqNo::from(1u32)).unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(2u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(2u32));
    let state = msg.state();
    assert_eq!(state.read("default", b"k0"), Some(b"first".to_vec()));
    assert_eq!(state.read("default", b"k1"), Some(b"second".to_vec()));
    assert_eq!(state.read("default", b"k2"), Some(b"third".to_vec()));
}

// ---------------------------------------------------------------------------
// Backtrack: conflicting speculation is corrected
// ---------------------------------------------------------------------------

/// Speculate seq 1 (k1="wrong") and seq 2 (k2="v2"), then receive seq 1 again
/// with k1="correct" — triggers backtrack.  On confirmation seq 1 must yield
/// the corrected value; seq 2 is discarded by the backtrack.
#[test]
fn test_backtrack_corrects_speculative_state() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(0, &[(b"k1".to_vec(), Some(b"wrong".to_vec()))]))
        .unwrap();
    handle
        .queue_preemptive_update(make_batch(1, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]))
        .unwrap();
    // seq 1 again with a different value — triggers internal backtrack.
    handle
        .queue_preemptive_update(make_batch(
            0,
            &[(b"k1".to_vec(), Some(b"correct".to_vec()))],
        ))
        .unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(0u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(0u32));
    let state = msg.state();
    assert_eq!(state.read("default", b"k1"), Some(b"correct".to_vec()));
    // k2 was discarded by the backtrack.
    assert!(state.read("default", b"k2").is_none());
}

/// After a backtrack the worker continues accepting and confirming new
/// preemptive updates from the corrected seq onwards.
#[test]
fn test_backtrack_then_continue_execution() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    // Speculate seq 1 (wrong) and seq 2.
    handle
        .queue_preemptive_update(make_batch(0, &[(b"k1".to_vec(), Some(b"wrong".to_vec()))]))
        .unwrap();
    handle
        .queue_preemptive_update(make_batch(1, &[(b"k2".to_vec(), Some(b"old".to_vec()))]))
        .unwrap();
    // Backtrack: seq 1 corrected.
    handle
        .queue_preemptive_update(make_batch(
            0,
            &[(b"k1".to_vec(), Some(b"correct".to_vec()))],
        ))
        .unwrap();
    // Re-execute seq 2 with the new value.
    handle
        .queue_preemptive_update(make_batch(1, &[(b"k2".to_vec(), Some(b"new".to_vec()))]))
        .unwrap();
    handle.queue_update_finalized(SeqNo::from(0u32)).unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(1u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(1u32));
    let state = msg.state();
    assert_eq!(state.read("default", b"k1"), Some(b"correct".to_vec()));
    assert_eq!(state.read("default", b"k2"), Some(b"new".to_vec()));
}

// ---------------------------------------------------------------------------
// Convergence: preemptive path == direct path
// ---------------------------------------------------------------------------

/// Path A: speculate seq 1–3 then confirm all.
/// Path B: direct seq 1–3.
/// Both must reach the same final state.
#[test]
fn test_convergence_write_sequence() {
    let ops: &[(&[u8], &[u8])] = &[(b"k1", b"v1"), (b"k2", b"v2"), (b"k3", b"v3")];

    // Path A: preemptive
    let (handle_a, _, chk_a) = spawn_worker();
    for (i, &(key, val)) in ops.iter().enumerate() {
        let seq = i as u32;
        handle_a
            .queue_preemptive_update(make_batch(seq, &[(key.to_vec(), Some(val.to_vec()))]))
            .unwrap();
    }
    handle_a.queue_update_finalized(SeqNo::from(0u32)).unwrap();
    handle_a.queue_update_finalized(SeqNo::from(1u32)).unwrap();
    handle_a
        .queue_update_finalized_and_get_appstate(SeqNo::from(2u32))
        .unwrap();
    let result_a = chk_a.recv_timeout(RECV_TIMEOUT).expect("path A timed out");

    // Path B: direct
    let (handle_b, _, chk_b) = spawn_worker();
    for (i, &(key, val)) in ops[..2].iter().enumerate() {
        let seq = i as u32;
        handle_b
            .queue_update(make_batch(seq, &[(key.to_vec(), Some(val.to_vec()))]))
            .unwrap();
    }
    let &(key, val) = &ops[2];
    handle_b
        .queue_update_and_get_appstate(make_batch(2, &[(key.to_vec(), Some(val.to_vec()))]))
        .unwrap();
    let result_b = chk_b.recv_timeout(RECV_TIMEOUT).expect("path B timed out");

    for &(key, _) in ops {
        assert_eq!(
            result_a.state().read("default", key),
            result_b.state().read("default", key),
            "states diverge for key {:?}",
            key
        );
    }
}

/// Convergence after a backtrack.
/// Path A: speculate seq 1 ("wrong") + seq 2, backtrack seq 1 to "correct", confirm seq 1.
/// Path B: directly confirm seq 1 with "correct".
/// Both must agree: k1="correct", k2 absent.
#[test]
fn test_convergence_after_backtrack() {
    // Path A: backtrack
    let (handle_a, _, chk_a) = spawn_worker();
    handle_a
        .queue_preemptive_update(make_batch(0, &[(b"k1".to_vec(), Some(b"wrong".to_vec()))]))
        .unwrap();
    handle_a
        .queue_preemptive_update(make_batch(1, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]))
        .unwrap();
    handle_a
        .queue_preemptive_update(make_batch(
            0,
            &[(b"k1".to_vec(), Some(b"correct".to_vec()))],
        ))
        .unwrap();
    handle_a
        .queue_update_finalized_and_get_appstate(SeqNo::from(0u32))
        .unwrap();
    let result_a = chk_a.recv_timeout(RECV_TIMEOUT).expect("path A timed out");

    // Path B: direct
    let (handle_b, _, chk_b) = spawn_worker();
    handle_b
        .queue_update_and_get_appstate(make_batch(
            0,
            &[(b"k1".to_vec(), Some(b"correct".to_vec()))],
        ))
        .unwrap();
    let result_b = chk_b.recv_timeout(RECV_TIMEOUT).expect("path B timed out");

    assert_eq!(
        result_a.state().read("default", b"k1"),
        result_b.state().read("default", b"k1"),
    );
    assert_eq!(
        result_a.state().read("default", b"k1"),
        Some(b"correct".to_vec())
    );
    assert!(result_a.state().read("default", b"k2").is_none());
    assert!(result_b.state().read("default", b"k2").is_none());
}

/// Convergence after a state transfer.
/// Install {base=installed} at seq 5, then write k1="after" at seq 6.
/// Path A: preemptive seq 6 then finalize.
/// Path B: direct seq 6.
#[test]
fn test_convergence_after_state_transfer() {
    fn install_state(
        handle: &crate::exec_handle::PreemptiveExecutorHandle<super::test_fixtures::KvOp>,
        state_tx: &atlas_common::channel::sync::ChannelSyncTx<InstallStateMessage<MapState>>,
    ) {
        let mut s = MapState::default();
        s.update("default", b"base", b"installed");
        handle.poll_state_channel().unwrap();
        state_tx
            .send(InstallStateMessage::new(SeqNo::from(5u32), s))
            .unwrap();
    }

    // Path A: preemptive seq 6 after state transfer.
    let (handle_a, state_tx_a, chk_a) = spawn_worker();
    install_state(&handle_a, &state_tx_a);
    handle_a
        .queue_preemptive_update(make_batch(6, &[(b"k1".to_vec(), Some(b"after".to_vec()))]))
        .unwrap();
    handle_a
        .queue_update_finalized_and_get_appstate(SeqNo::from(6u32))
        .unwrap();
    let result_a = chk_a.recv_timeout(RECV_TIMEOUT).expect("path A timed out");

    // Path B: direct seq 6 after state transfer.
    let (handle_b, state_tx_b, chk_b) = spawn_worker();
    install_state(&handle_b, &state_tx_b);
    handle_b
        .queue_update_and_get_appstate(make_batch(6, &[(b"k1".to_vec(), Some(b"after".to_vec()))]))
        .unwrap();
    let result_b = chk_b.recv_timeout(RECV_TIMEOUT).expect("path B timed out");

    for key in [b"base" as &[u8], b"k1"] {
        assert_eq!(
            result_a.state().read("default", key),
            result_b.state().read("default", key),
            "states diverge for key {:?}",
            key
        );
    }
    assert_eq!(
        result_a.state().read("default", b"k1"),
        Some(b"after".to_vec())
    );
    assert_eq!(
        result_a.state().read("default", b"base"),
        Some(b"installed".to_vec())
    );
}
