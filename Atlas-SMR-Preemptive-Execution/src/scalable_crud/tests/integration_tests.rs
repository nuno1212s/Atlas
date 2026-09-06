//! End-to-end integration tests for the scalable CRUD preemptive executor.
//!
//! Each test drives the worker through its real channel API
//! (`PreemptiveExecutorHandle`) and observes confirmed-state changes via the
//! checkpoint receiver the worker emits for `*AndGetAppstate` variants.
//!
//! # Convergence invariant
//!
//! Several tests run the same operations through two independent worker instances:
//!
//! * **Path A** — scalable preemptive speculation + finalization.
//! * **Path B** — directly-confirmed execution (no speculation).
//!
//! Both paths must reach the same confirmed state.

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

// ---------------------------------------------------------------------------
// Tombstone: delete through the cache reaches confirmed state
// ---------------------------------------------------------------------------

#[test]
fn test_tombstone_delete_confirmed() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_update(make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]))
        .unwrap();
    handle
        .queue_preemptive_update(make_batch(2, &[(b"k1".to_vec(), None)]))
        .unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(2u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(2u32));
    assert!(msg.state().read("default", b"k1").is_none());
}

// ---------------------------------------------------------------------------
// Collision: batch with conflicting ops on the same key
// ---------------------------------------------------------------------------

/// A batch where two ops write to the same key triggers a collision.
/// After sequential re-execution the last op in batch order wins.
#[test]
fn test_collision_write_write_confirmed_state_correct() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(
            0,
            &[
                (b"k1".to_vec(), Some(b"first".to_vec())),
                (b"k1".to_vec(), Some(b"second".to_vec())),
            ],
        ))
        .unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(0u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(0u32));
    // The second op in the batch (which executes after the first in sequential re-exec) wins.
    assert_eq!(msg.state().read("default", b"k1"), Some(b"second".to_vec()));
}

/// op0 writes k1=written, op1 deletes k1 — collision → sequential: k1 is deleted.
#[test]
fn test_collision_write_then_delete_confirmed_state_correct() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(
            0,
            &[
                (b"k1".to_vec(), Some(b"written".to_vec())),
                (b"k1".to_vec(), None),
            ],
        ))
        .unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(0u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(0u32));
    assert!(msg.state().read("default", b"k1").is_none());
}

/// Multi-op batch: ops on k1, k3 are independent; ops on k2 collide.
/// After finalization all keys must have the correct (sequentially-ordered) values.
#[test]
fn test_collision_mixed_batch_correct_state() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(
            0,
            &[
                (b"k1".to_vec(), Some(b"v1".to_vec())),
                (b"k2".to_vec(), Some(b"v2_a".to_vec())),
                (b"k2".to_vec(), Some(b"v2_b".to_vec())),
                (b"k3".to_vec(), Some(b"v3".to_vec())),
            ],
        ))
        .unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(0u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    let state = msg.state();
    assert_eq!(state.read("default", b"k1"), Some(b"v1".to_vec()));
    assert_eq!(state.read("default", b"k2"), Some(b"v2_b".to_vec()));
    assert_eq!(state.read("default", b"k3"), Some(b"v3".to_vec()));
}

// ---------------------------------------------------------------------------
// CatchUp
// ---------------------------------------------------------------------------

#[test]
fn test_catch_up_discards_pending_and_applies_batches() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(
            1,
            &[(b"speculative".to_vec(), Some(b"gone".to_vec()))],
        ))
        .unwrap();

    handle
        .catch_up_to_quorum(MaybeVec::from_many(vec![
            make_batch(1, &[(b"k1".to_vec(), Some(b"caught".to_vec()))]),
            make_batch(2, &[(b"k2".to_vec(), Some(b"up".to_vec()))]),
        ]))
        .unwrap();

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

#[test]
fn test_state_transfer_then_resume() {
    let (handle, state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(1, &[(b"old".to_vec(), Some(b"data".to_vec()))]))
        .unwrap();

    handle.poll_state_channel().unwrap();

    let mut installed = MapState::default();
    installed.update("default", b"base", b"installed");
    state_tx
        .send(InstallStateMessage::new(SeqNo::from(5u32), installed))
        .expect("state install send failed");

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

#[test]
fn test_backtrack_corrects_speculative_state() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    handle
        .queue_preemptive_update(make_batch(0, &[(b"k1".to_vec(), Some(b"wrong".to_vec()))]))
        .unwrap();
    handle
        .queue_preemptive_update(make_batch(1, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]))
        .unwrap();
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
    assert!(state.read("default", b"k2").is_none());
}

/// Backtrack after a batch that had internal collisions.
#[test]
fn test_backtrack_after_collision_batch() {
    let (handle, _state_tx, checkpoint_rx) = spawn_worker();

    // seq 1: two ops on k1 (collision), both re-executed sequentially.
    handle
        .queue_preemptive_update(make_batch(
            0,
            &[
                (b"k1".to_vec(), Some(b"v1_a".to_vec())),
                (b"k1".to_vec(), Some(b"v1_b".to_vec())),
            ],
        ))
        .unwrap();
    // seq 2: write k2.
    handle
        .queue_preemptive_update(make_batch(1, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]))
        .unwrap();
    // Backtrack to seq 2 (discard seq 2, keep seq 1).
    handle
        .queue_preemptive_update(make_batch(
            1,
            &[(b"k2".to_vec(), Some(b"v2_corrected".to_vec()))],
        ))
        .unwrap();
    handle.queue_update_finalized(SeqNo::from(0u32)).unwrap();
    handle
        .queue_update_finalized_and_get_appstate(SeqNo::from(1u32))
        .unwrap();

    let msg = checkpoint_rx.recv_timeout(RECV_TIMEOUT).expect("timed out");
    assert_eq!(msg.seq(), SeqNo::from(1u32));
    let state = msg.state();
    assert_eq!(state.read("default", b"k1"), Some(b"v1_b".to_vec()));
    assert_eq!(state.read("default", b"k2"), Some(b"v2_corrected".to_vec()));
}

// ---------------------------------------------------------------------------
// Convergence: scalable preemptive path == direct path
// ---------------------------------------------------------------------------

#[test]
fn test_convergence_write_sequence() {
    let ops: &[(&[u8], &[u8])] = &[(b"k1", b"v1"), (b"k2", b"v2"), (b"k3", b"v3")];

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

/// Convergence: parallel batch with write-write collision vs. direct sequential.
#[test]
fn test_convergence_collision_batch_vs_direct() {
    let ops: &[(Vec<u8>, Option<Vec<u8>>)] = &[
        (b"k1".to_vec(), Some(b"v1".to_vec())),
        (b"k2".to_vec(), Some(b"v2_a".to_vec())),
        (b"k2".to_vec(), Some(b"v2_b".to_vec())), // collision on k2
        (b"k3".to_vec(), Some(b"v3".to_vec())),
    ];

    // Path A: scalable preemptive.
    let (handle_a, _, chk_a) = spawn_worker();
    handle_a
        .queue_preemptive_update(make_batch(0, ops))
        .unwrap();
    handle_a
        .queue_update_finalized_and_get_appstate(SeqNo::from(0u32))
        .unwrap();
    let result_a = chk_a.recv_timeout(RECV_TIMEOUT).expect("path A timed out");

    // Path B: direct confirmed (identical batch).
    let (handle_b, _, chk_b) = spawn_worker();
    handle_b
        .queue_update_and_get_appstate(make_batch(0, ops))
        .unwrap();
    let result_b = chk_b.recv_timeout(RECV_TIMEOUT).expect("path B timed out");

    for key in [b"k1" as &[u8], b"k2", b"k3"] {
        assert_eq!(
            result_a.state().read("default", key),
            result_b.state().read("default", key),
            "states diverge for key {:?}",
            key
        );
    }
}

/// Convergence after a backtrack.
#[test]
fn test_convergence_after_backtrack() {
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

    let (handle_a, state_tx_a, chk_a) = spawn_worker();
    install_state(&handle_a, &state_tx_a);
    handle_a
        .queue_preemptive_update(make_batch(6, &[(b"k1".to_vec(), Some(b"after".to_vec()))]))
        .unwrap();
    handle_a
        .queue_update_finalized_and_get_appstate(SeqNo::from(6u32))
        .unwrap();
    let result_a = chk_a.recv_timeout(RECV_TIMEOUT).expect("path A timed out");

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
