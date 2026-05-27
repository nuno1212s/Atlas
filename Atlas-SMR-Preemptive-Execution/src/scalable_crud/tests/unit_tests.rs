//! Unit tests for `ScalableCachingPreemptiveState`.
//!
//! These tests exercise the state-machine logic directly — no channels, no threads.
//! They cover:
//!   1. All baseline behaviours from `single_threaded_crud` (preemptive, confirm, backtrack,
//!      catch-up, state-install).
//!   2. Collision-specific scenarios unique to the parallel executor.

use atlas_common::ordering::SeqNo;
use atlas_smr_execution::crud_states::CRUDState;
use rayon::ThreadPoolBuilder;

use super::test_fixtures::{MapApp, MapState, make_batch};
use crate::scalable_crud::pending_state::{BacktrackError, ScalableCachingPreemptiveState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn new_state() -> ScalableCachingPreemptiveState<MapState, MapApp> {
    let pool = ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    ScalableCachingPreemptiveState::new((SeqNo::ZERO, MapState::default()), pool)
}

fn read_confirmed(
    state: &ScalableCachingPreemptiveState<MapState, MapApp>,
    key: &[u8],
) -> Option<Vec<u8>> {
    state.confirmed_state().read("default", key)
}

// ---------------------------------------------------------------------------
// Initial state
// ---------------------------------------------------------------------------

#[test]
fn test_initial_state() {
    let s = new_state();
    assert_eq!(s.confirmed_seq_no(), SeqNo::ZERO);
    assert_eq!(s.preemptive_seq_no(), SeqNo::ZERO);
}

// ---------------------------------------------------------------------------
// Preemptive updates accumulate in the cache
// ---------------------------------------------------------------------------

#[test]
fn test_preemptive_update_advances_preemptive_seq() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    assert_eq!(s.preemptive_seq_no(), SeqNo::from(1u32));
    assert_eq!(s.confirmed_seq_no(), SeqNo::ZERO);
    assert!(read_confirmed(&s, b"k1").is_none());
}

#[test]
fn test_accumulated_cache_visible_to_next_preemptive() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    s.handle_preemptive_update(
        &app,
        make_batch(2, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]),
    )
    .unwrap();
    assert!(read_confirmed(&s, b"k1").is_none());
    assert!(read_confirmed(&s, b"k2").is_none());
    assert_eq!(s.preemptive_seq_no(), SeqNo::from(2u32));
}

// ---------------------------------------------------------------------------
// Confirmation: delta applied to real state
// ---------------------------------------------------------------------------

#[test]
fn test_confirmation_applies_delta_to_real_state() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    let replies = s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    assert_eq!(s.confirmed_seq_no(), SeqNo::from(1u32));
}

#[test]
fn test_accumulated_cache_rebuilt_after_confirmation() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    s.handle_preemptive_update(
        &app,
        make_batch(2, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]),
    )
    .unwrap();
    s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    assert!(read_confirmed(&s, b"k2").is_none());
    s.handle_update_confirmed(SeqNo::from(2u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"v2".to_vec()));
}

// ---------------------------------------------------------------------------
// Tombstone semantics
// ---------------------------------------------------------------------------

#[test]
fn test_delete_tombstone_shadows_confirmed_state() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_confirmed_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    s.handle_preemptive_update(&app, make_batch(2, &[(b"k1".to_vec(), None)]))
        .unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    s.handle_update_confirmed(SeqNo::from(2u32)).unwrap();
    assert!(read_confirmed(&s, b"k1").is_none());
}

// ---------------------------------------------------------------------------
// Backtrack
// ---------------------------------------------------------------------------

#[test]
fn test_backtrack_discards_correct_entries() {
    let app = MapApp;
    let mut s = new_state();
    for (seq, key) in [(1, b"k1" as &[u8]), (2, b"k2"), (3, b"k3")] {
        s.handle_preemptive_update(
            &app,
            make_batch(seq, &[(key.to_vec(), Some(b"v".to_vec()))]),
        )
        .unwrap();
    }
    s.backtrack(SeqNo::from(2u32)).unwrap();
    assert_eq!(s.preemptive_seq_no(), SeqNo::from(1u32));
    assert_eq!(s.pending_count(), 1);
    assert_eq!(s.pending_front_seq(), Some(SeqNo::from(1u32)));
    assert!(read_confirmed(&s, b"k1").is_none());
}

#[test]
fn test_backtrack_to_confirmed_returns_error() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    let err = s.backtrack(SeqNo::from(1u32)).unwrap_err();
    assert!(matches!(
        err,
        BacktrackError::BacktrackToConfirmedOrBelow { .. }
    ));
}

#[test]
fn test_backtrack_then_re_execute() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"original".to_vec()))]),
    )
    .unwrap();
    s.handle_preemptive_update(
        &app,
        make_batch(2, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]),
    )
    .unwrap();
    s.backtrack(SeqNo::from(2u32)).unwrap();
    s.handle_preemptive_update(
        &app,
        make_batch(2, &[(b"k2".to_vec(), Some(b"corrected".to_vec()))]),
    )
    .unwrap();
    s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    s.handle_update_confirmed(SeqNo::from(2u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"original".to_vec()));
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"corrected".to_vec()));
}

// ---------------------------------------------------------------------------
// CatchUp
// ---------------------------------------------------------------------------

#[test]
fn test_catch_up_clears_pending_and_updates_real_state() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"speculative".to_vec(), Some(b"gone".to_vec()))]),
    )
    .unwrap();
    let results = s.handle_catch_up(
        &app,
        vec![
            make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
            make_batch(2, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]),
        ],
    );
    assert_eq!(results.len(), 2);
    assert_eq!(s.confirmed_seq_no(), SeqNo::from(2u32));
    assert_eq!(s.preemptive_seq_no(), SeqNo::from(2u32));
    assert_eq!(s.pending_count(), 0);
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"v2".to_vec()));
    assert!(read_confirmed(&s, b"speculative").is_none());
}

// ---------------------------------------------------------------------------
// Install state
// ---------------------------------------------------------------------------

#[test]
fn test_install_confirmed_state_resets_everything() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    let mut new_inner = MapState::default();
    new_inner.update("default", b"external", b"value");
    s.install_confirmed_state(SeqNo::from(10u32), new_inner);
    assert_eq!(s.confirmed_seq_no(), SeqNo::from(10u32));
    assert_eq!(s.preemptive_seq_no(), SeqNo::from(10u32));
    assert_eq!(s.pending_count(), 0);
    assert_eq!(read_confirmed(&s, b"external"), Some(b"value".to_vec()));
    assert!(read_confirmed(&s, b"k1").is_none());
}

// ---------------------------------------------------------------------------
// Collision-specific tests
// ---------------------------------------------------------------------------

/// Two ops on different keys within the same batch should not collide.
/// Both writes must be present in the confirmed state after finalization.
#[test]
fn test_no_collision_parallel_correctness() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(
            1,
            &[
                (b"k1".to_vec(), Some(b"v1".to_vec())),
                (b"k2".to_vec(), Some(b"v2".to_vec())),
                (b"k3".to_vec(), Some(b"v3".to_vec())),
            ],
        ),
    )
    .unwrap();
    s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"v2".to_vec()));
    assert_eq!(read_confirmed(&s, b"k3"), Some(b"v3".to_vec()));
}

/// Two ops writing to the same key collide and are re-executed sequentially.
/// The later op in batch order wins (op1 comes after op0).
#[test]
fn test_write_write_collision_last_writer_wins() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(
            1,
            &[
                (b"k1".to_vec(), Some(b"v_op0".to_vec())),
                (b"k1".to_vec(), Some(b"v_op1".to_vec())),
            ],
        ),
    )
    .unwrap();
    s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    // op1 executes after op0, so op1 overwrites op0.
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v_op1".to_vec()));
}

/// op0 writes k1, op1 deletes k1. Both collide.
/// After sequential re-execution (op0 first, then op1): k1 ends up deleted.
#[test]
fn test_write_then_delete_collision() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(
            1,
            &[
                (b"k1".to_vec(), Some(b"written".to_vec())),
                (b"k1".to_vec(), None),
            ],
        ),
    )
    .unwrap();
    s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    assert!(read_confirmed(&s, b"k1").is_none());
}

/// op0 deletes k1 (already exists), op1 writes k1. Both collide.
/// After sequential re-execution: op0 deletes, then op1 writes → k1=new_value.
#[test]
fn test_delete_then_write_collision() {
    let app = MapApp;
    let mut s = new_state();
    // Establish k1 in confirmed state.
    s.handle_confirmed_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"original".to_vec()))]),
    )
    .unwrap();
    // Batch: delete k1, then write k1=new.
    s.handle_preemptive_update(
        &app,
        make_batch(
            2,
            &[
                (b"k1".to_vec(), None),
                (b"k1".to_vec(), Some(b"new".to_vec())),
            ],
        ),
    )
    .unwrap();
    s.handle_update_confirmed(SeqNo::from(2u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"new".to_vec()));
}

/// Mixed batch: some non-colliding and some colliding ops.
/// k1 and k3 are independent; k2 is written by two ops (collision).
#[test]
fn test_mixed_collision_and_no_collision() {
    let app = MapApp;
    let mut s = new_state();
    s.handle_preemptive_update(
        &app,
        make_batch(
            1,
            &[
                (b"k1".to_vec(), Some(b"v1".to_vec())),        // no collision
                (b"k2".to_vec(), Some(b"v2_first".to_vec())),  // collides with op2
                (b"k2".to_vec(), Some(b"v2_second".to_vec())), // collides with op1
                (b"k3".to_vec(), Some(b"v3".to_vec())),        // no collision
            ],
        ),
    )
    .unwrap();
    s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"v2_second".to_vec()));
    assert_eq!(read_confirmed(&s, b"k3"), Some(b"v3".to_vec()));
}

/// Convergence: the scalable executor must produce the same confirmed state
/// as sequential execution of the same batch.
#[test]
fn test_collision_convergence_with_sequential() {
    use crate::single_threaded_crud::pending_state::CachingPreemptiveState;

    let ops: &[(Vec<u8>, Option<Vec<u8>>)] = &[
        (b"k1".to_vec(), Some(b"v1".to_vec())),
        (b"k2".to_vec(), Some(b"v2_first".to_vec())),
        (b"k2".to_vec(), Some(b"v2_second".to_vec())), // collision on k2
        (b"k3".to_vec(), Some(b"v3".to_vec())),
    ];

    // Path A: scalable executor.
    let app = MapApp;
    let mut scalable = new_state();
    scalable
        .handle_preemptive_update(&app, make_batch(1, ops))
        .unwrap();
    scalable.handle_update_confirmed(SeqNo::from(1u32)).unwrap();

    // Path B: sequential single-threaded CRUD executor.
    let mut sequential = CachingPreemptiveState::new((SeqNo::ZERO, MapState::default()));
    sequential
        .handle_preemptive_update(&app, make_batch(1, ops))
        .unwrap();
    sequential
        .handle_update_confirmed(SeqNo::from(1u32))
        .unwrap();

    // Both must agree on the confirmed state.
    for key in [b"k1" as &[u8], b"k2", b"k3"] {
        assert_eq!(
            scalable.confirmed_state().read("default", key),
            sequential.confirmed_state().read("default", key),
            "states diverge for key {:?}",
            key
        );
    }
}

/// Convergence across multiple batches with cross-batch accumulated cache interactions.
#[test]
fn test_multi_batch_convergence() {
    use crate::single_threaded_crud::pending_state::CachingPreemptiveState;

    let app = MapApp;

    let batch1 = &[
        (b"k1".to_vec(), Some(b"a".to_vec())),
        (b"k2".to_vec(), Some(b"b".to_vec())),
    ];
    let batch2 = &[
        (b"k1".to_vec(), Some(b"c".to_vec())), // overwrites k1
        (b"k3".to_vec(), Some(b"d".to_vec())),
    ];

    let mut scalable = new_state();
    scalable
        .handle_preemptive_update(&app, make_batch(1, batch1))
        .unwrap();
    scalable
        .handle_preemptive_update(&app, make_batch(2, batch2))
        .unwrap();
    scalable.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    scalable.handle_update_confirmed(SeqNo::from(2u32)).unwrap();

    let mut sequential = CachingPreemptiveState::new((SeqNo::ZERO, MapState::default()));
    sequential
        .handle_preemptive_update(&app, make_batch(1, batch1))
        .unwrap();
    sequential
        .handle_preemptive_update(&app, make_batch(2, batch2))
        .unwrap();
    sequential
        .handle_update_confirmed(SeqNo::from(1u32))
        .unwrap();
    sequential
        .handle_update_confirmed(SeqNo::from(2u32))
        .unwrap();

    for key in [b"k1" as &[u8], b"k2", b"k3"] {
        assert_eq!(
            scalable.confirmed_state().read("default", key),
            sequential.confirmed_state().read("default", key),
            "states diverge for key {:?}",
            key
        );
    }
}

/// Mirrors the integration test `test_backtrack_after_collision_batch`.
/// seq=1 is a collision batch (two writes to k1), then seq=2 is speculated,
/// then seq=2 backtracks (re-executes with corrected value).
#[test]
fn test_backtrack_after_collision_batch() {
    let app = MapApp;
    let mut s = new_state();

    // seq=1: collision batch — two writes to k1.
    s.handle_preemptive_update(
        &app,
        make_batch(
            1,
            &[
                (b"k1".to_vec(), Some(b"v1_a".to_vec())),
                (b"k1".to_vec(), Some(b"v1_b".to_vec())),
            ],
        ),
    )
    .unwrap();

    // seq=2: write k2=v2.
    s.handle_preemptive_update(
        &app,
        make_batch(2, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]),
    )
    .unwrap();

    // Backtrack: discard seq=2, keep seq=1.
    s.backtrack(SeqNo::from(2u32)).unwrap();

    // Re-execute seq=2 with corrected value.
    s.handle_preemptive_update(
        &app,
        make_batch(2, &[(b"k2".to_vec(), Some(b"v2_corrected".to_vec()))]),
    )
    .unwrap();

    // Confirm seq=1, then seq=2.
    s.handle_update_confirmed(SeqNo::from(1u32)).unwrap();
    s.handle_update_confirmed(SeqNo::from(2u32)).unwrap();

    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1_b".to_vec()));
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"v2_corrected".to_vec()));
}
