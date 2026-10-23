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
use crate::single_threaded_crud::caching_state::{CachingState, cache_entry_count};
use crate::single_threaded_crud::pending_state::CACHE_COMPACTION_FACTOR;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A freshly booted state: nothing confirmed, and the next batch it expects is `SeqNo::ZERO`
/// — which is what consensus actually delivers first.
fn new_state() -> ScalableCachingPreemptiveState<MapState, MapApp> {
    new_state_expecting(SeqNo::ZERO)
}

/// A state positioned so that the next batch it expects is `seq`. Most tests below use
/// 1-based sequences for readability.
fn new_state_expecting(seq: SeqNo) -> ScalableCachingPreemptiveState<MapState, MapApp> {
    let pool = ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    ScalableCachingPreemptiveState::new((seq, MapState::default()), pool)
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
    assert_eq!(s.next_confirmed_seq(), SeqNo::ZERO);
    assert_eq!(s.next_preemptive_seq(), SeqNo::ZERO);
}

// ---------------------------------------------------------------------------
// Preemptive updates accumulate in the cache
// ---------------------------------------------------------------------------

#[test]
fn test_preemptive_update_advances_preemptive_seq() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(2u32));
    assert_eq!(s.next_confirmed_seq(), SeqNo::ONE);
    assert!(read_confirmed(&s, b"k1").is_none());
}

#[test]
fn test_accumulated_cache_visible_to_next_preemptive() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
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
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(3u32));
}

// ---------------------------------------------------------------------------
// Confirmation: delta applied to real state
// ---------------------------------------------------------------------------

#[test]
fn test_confirmation_applies_delta_to_real_state() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    let replies = s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    assert_eq!(s.next_confirmed_seq(), SeqNo::from(2u32));
}

#[test]
fn test_confirmation_keeps_later_pending_writes_pending() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
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
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    assert!(read_confirmed(&s, b"k2").is_none());
    s.handle_update_confirmed(&app, SeqNo::from(2u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"v2".to_vec()));
}

// ---------------------------------------------------------------------------
// Tombstone semantics
// ---------------------------------------------------------------------------

#[test]
fn test_delete_tombstone_shadows_confirmed_state() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
    s.handle_confirmed_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    s.handle_preemptive_update(&app, make_batch(2, &[(b"k1".to_vec(), None)]))
        .unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    s.handle_update_confirmed(&app, SeqNo::from(2u32)).unwrap();
    assert!(read_confirmed(&s, b"k1").is_none());
}

// ---------------------------------------------------------------------------
// Backtrack
// ---------------------------------------------------------------------------

#[test]
fn test_backtrack_discards_correct_entries() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
    for (seq, key) in [(1, b"k1" as &[u8]), (2, b"k2"), (3, b"k3")] {
        s.handle_preemptive_update(
            &app,
            make_batch(seq, &[(key.to_vec(), Some(b"v".to_vec()))]),
        )
        .unwrap();
    }
    s.backtrack(SeqNo::from(2u32)).unwrap();
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(2u32));
    assert_eq!(s.pending_count(), 1);
    assert_eq!(s.pending_front_seq(), Some(SeqNo::from(1u32)));
    assert!(read_confirmed(&s, b"k1").is_none());
}

#[test]
fn test_backtrack_to_confirmed_returns_error() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    let err = s.backtrack(SeqNo::from(1u32)).unwrap_err();
    assert!(matches!(
        err,
        BacktrackError::BacktrackBelowConfirmed { .. }
    ));
}

#[test]
fn test_backtrack_then_re_execute() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
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
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    s.handle_update_confirmed(&app, SeqNo::from(2u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"original".to_vec()));
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"corrected".to_vec()));
}

// ---------------------------------------------------------------------------
// CatchUp
// ---------------------------------------------------------------------------

#[test]
fn test_catch_up_clears_pending_and_updates_real_state() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
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
    assert_eq!(s.next_confirmed_seq(), SeqNo::from(3u32));
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(3u32));
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
    let mut s = new_state_expecting(SeqNo::ONE);
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    let mut new_inner = MapState::default();
    new_inner.update("default", b"external", b"value");
    s.install_confirmed_state(SeqNo::from(10u32), new_inner);
    assert_eq!(s.next_confirmed_seq(), SeqNo::from(11u32));
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(11u32));
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
    let mut s = new_state_expecting(SeqNo::ONE);
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
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"v2".to_vec()));
    assert_eq!(read_confirmed(&s, b"k3"), Some(b"v3".to_vec()));
}

/// Two ops writing to the same key collide and are re-executed sequentially.
/// The later op in batch order wins (op1 comes after op0).
#[test]
fn test_write_write_collision_last_writer_wins() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
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
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    // op1 executes after op0, so op1 overwrites op0.
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v_op1".to_vec()));
}

/// op0 writes k1, op1 deletes k1. Both collide.
/// After sequential re-execution (op0 first, then op1): k1 ends up deleted.
#[test]
fn test_write_then_delete_collision() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
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
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    assert!(read_confirmed(&s, b"k1").is_none());
}

/// op0 deletes k1 (already exists), op1 writes k1. Both collide.
/// After sequential re-execution: op0 deletes, then op1 writes → k1=new_value.
#[test]
fn test_delete_then_write_collision() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
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
    s.handle_update_confirmed(&app, SeqNo::from(2u32)).unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"new".to_vec()));
}

/// Mixed batch: some non-colliding and some colliding ops.
/// k1 and k3 are independent; k2 is written by two ops (collision).
#[test]
fn test_mixed_collision_and_no_collision() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
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
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
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
    let mut scalable = new_state_expecting(SeqNo::ONE);
    scalable
        .handle_preemptive_update(&app, make_batch(1, ops))
        .unwrap();
    scalable
        .handle_update_confirmed(&app, SeqNo::from(1u32))
        .unwrap();

    // Path B: sequential single-threaded CRUD executor.
    let mut sequential = CachingPreemptiveState::new((SeqNo::ONE, MapState::default()));
    sequential
        .handle_preemptive_update(&app, make_batch(1, ops))
        .unwrap();
    sequential
        .handle_update_confirmed(&app, SeqNo::from(1u32))
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

    let mut scalable = new_state_expecting(SeqNo::ONE);
    scalable
        .handle_preemptive_update(&app, make_batch(1, batch1))
        .unwrap();
    scalable
        .handle_preemptive_update(&app, make_batch(2, batch2))
        .unwrap();
    scalable
        .handle_update_confirmed(&app, SeqNo::from(1u32))
        .unwrap();
    scalable
        .handle_update_confirmed(&app, SeqNo::from(2u32))
        .unwrap();

    let mut sequential = CachingPreemptiveState::new((SeqNo::ONE, MapState::default()));
    sequential
        .handle_preemptive_update(&app, make_batch(1, batch1))
        .unwrap();
    sequential
        .handle_preemptive_update(&app, make_batch(2, batch2))
        .unwrap();
    sequential
        .handle_update_confirmed(&app, SeqNo::from(1u32))
        .unwrap();
    sequential
        .handle_update_confirmed(&app, SeqNo::from(2u32))
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
    let mut s = new_state_expecting(SeqNo::ONE);

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
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    s.handle_update_confirmed(&app, SeqNo::from(2u32)).unwrap();

    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1_b".to_vec()));
    assert_eq!(read_confirmed(&s, b"k2"), Some(b"v2_corrected".to_vec()));
}

// ---------------------------------------------------------------------------
// The cache is not refolded on confirmation
// ---------------------------------------------------------------------------

fn write(key: &[u8], value: &[u8]) -> (Vec<u8>, Option<Vec<u8>>) {
    (key.to_vec(), Some(value.to_vec()))
}

fn delete(key: &[u8]) -> (Vec<u8>, Option<Vec<u8>>) {
    (key.to_vec(), None)
}

/// For every key, reading through the cache and then the confirmed state gives the same
/// answer as reading through a full refold of the pending deltas and then the confirmed state.
/// This is the invariant that replaces `cache == fold(pending)`.
fn assert_read_equivalent(s: &ScalableCachingPreemptiveState<MapState, MapApp>, keys: &[&[u8]]) {
    let fold = s.pending_fold();
    for key in keys {
        let via_cache =
            CachingState::new(s.confirmed_state(), s.accumulated_cache()).read("default", key);
        let via_fold = CachingState::new(s.confirmed_state(), &fold).read("default", key);
        assert_eq!(
            via_cache,
            via_fold,
            "key {:?} reads differently through the cache",
            String::from_utf8_lossy(key)
        );
    }
}

#[test]
fn test_cache_read_equivalent_to_pending_fold_after_confirms() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
    let keys: &[&[u8]] = &[b"a", b"b", b"c"];

    fn speculate(
        s: &mut ScalableCachingPreemptiveState<MapState, MapApp>,
        seq: u32,
        ops: &[(Vec<u8>, Option<Vec<u8>>)],
    ) {
        s.handle_preemptive_update(&MapApp, make_batch(seq, ops))
            .unwrap();
        assert_read_equivalent(s, &[b"a", b"b", b"c"]);
    }

    speculate(&mut s, 1, &[write(b"a", b"1"), write(b"b", b"1")]);
    speculate(&mut s, 2, &[write(b"a", b"2"), write(b"c", b"2")]);

    // a=1 and b=1 stay in the cache: a is overridden by seq 2, b equals the confirmed value.
    s.handle_update_confirmed(&app, SeqNo::from(1u32)).unwrap();
    assert_read_equivalent(&s, keys);

    speculate(&mut s, 3, &[delete(b"b"), write(b"c", b"3")]);
    s.handle_update_confirmed(&app, SeqNo::from(2u32)).unwrap();
    assert_read_equivalent(&s, keys);

    // A backtrack drops pending entries and has to refold.
    speculate(&mut s, 4, &[write(b"a", b"wrong")]);
    s.backtrack(SeqNo::from(4u32)).unwrap();
    assert_read_equivalent(&s, keys);

    // Re-create b after its pending delete, and delete a.
    speculate(&mut s, 4, &[write(b"b", b"4"), delete(b"a")]);
    // Confirming the delete leaves a tombstone in the cache and removes b from the state.
    s.handle_update_confirmed(&app, SeqNo::from(3u32)).unwrap();
    assert!(read_confirmed(&s, b"b").is_none());
    assert_read_equivalent(&s, keys);

    speculate(&mut s, 5, &[delete(b"c")]);
    s.handle_update_confirmed(&app, SeqNo::from(4u32)).unwrap();
    assert_read_equivalent(&s, keys);

    s.handle_update_confirmed(&app, SeqNo::from(5u32)).unwrap();
    assert_read_equivalent(&s, keys);

    // The log drained, so the cache must be empty rather than full of confirmed entries.
    assert_eq!(s.pending_count(), 0);
    assert!(s.accumulated_cache().is_empty());

    assert!(read_confirmed(&s, b"a").is_none());
    assert_eq!(read_confirmed(&s, b"b"), Some(b"4".to_vec()));
    assert!(read_confirmed(&s, b"c").is_none());
}

/// A write that reaches the confirmed state without a pending delta must not be shadowed by
/// cache entries left over from earlier confirmations.
#[test]
fn test_direct_update_after_drain_is_not_shadowed() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);

    s.handle_preemptive_update(&app, make_batch(1, &[write(b"k", b"v1")]))
        .unwrap();
    s.handle_update_confirmed(&app, SeqNo::ONE).unwrap();
    assert!(s.accumulated_cache().is_empty());

    s.handle_confirmed_update(&app, make_batch(2, &[write(b"k", b"v2")]))
        .unwrap();
    assert_eq!(read_confirmed(&s, b"k"), Some(b"v2".to_vec()));

    // MapApp replies with the previous value, so this reply is what speculation read for k.
    s.handle_preemptive_update(&app, make_batch(3, &[write(b"k", b"v3")]))
        .unwrap();
    let replies = s.handle_update_confirmed(&app, SeqNo::from(3u32)).unwrap();
    let replies = replies.into_inner();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].reply(), &Some(b"v2".to_vec()));
    assert_eq!(read_confirmed(&s, b"k"), Some(b"v3".to_vec()));
}

/// With the log never draining, confirmed entries must not pile up in the cache forever.
#[test]
fn test_cache_compaction_bounds_growth() {
    let app = MapApp;
    let mut s = new_state_expecting(SeqNo::ONE);
    let keys: Vec<Vec<u8>> = (0..=50u32).map(|i| format!("k{i}").into_bytes()).collect();
    let key_refs: Vec<&[u8]> = keys.iter().map(|k| k.as_slice()).collect();

    s.handle_preemptive_update(&app, make_batch(1, &[write(&keys[0], b"v")]))
        .unwrap();

    let mut kept_confirmed_entries = false;
    for i in 1..=50u32 {
        s.handle_preemptive_update(&app, make_batch(i + 1, &[write(&keys[i as usize], b"v")]))
            .unwrap();
        s.handle_update_confirmed(&app, SeqNo::from(i)).unwrap();

        assert_eq!(s.pending_count(), 1);
        assert_eq!(s.pending_entries(), cache_entry_count(&s.pending_fold()));

        let cached = cache_entry_count(s.accumulated_cache());
        assert!(
            cached <= CACHE_COMPACTION_FACTOR * s.pending_entries(),
            "cache holds {cached} entries for {} pending",
            s.pending_entries()
        );
        kept_confirmed_entries |= cached > s.pending_entries();

        assert_read_equivalent(&s, &key_refs);
    }

    // Not every confirmation refolds: some of them leave confirmed entries behind.
    assert!(kept_confirmed_entries);
}
