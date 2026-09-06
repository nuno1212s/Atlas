//! Unit tests for `CachingPreemptiveState`.
//!
//! These tests exercise the state-machine logic directly — no channels, no threads.

use atlas_common::ordering::SeqNo;
use atlas_smr_execution::crud_states::CRUDState;

use super::test_fixtures::{MapApp, MapState, make_batch};
use crate::single_threaded_crud::pending_state::{
    BacktrackError, CachingPreemptiveState, PreemptiveOutcome,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A freshly booted state: nothing confirmed, and the next batch it expects is `SeqNo::ZERO`
/// — which is what consensus actually delivers first.
fn new_state() -> CachingPreemptiveState<MapState, MapApp> {
    CachingPreemptiveState::new((SeqNo::ZERO, MapState::default()))
}

/// A state positioned so that the next batch it expects is `seq`. Most tests below use
/// 1-based sequences for readability; the fresh-boot case where the first batch is seq 0 is
/// covered separately by the reorder/first-batch tests.
fn new_state_expecting(seq: SeqNo) -> CachingPreemptiveState<MapState, MapApp> {
    CachingPreemptiveState::new((seq, MapState::default()))
}

fn read_confirmed(state: &CachingPreemptiveState<MapState, MapApp>, key: &[u8]) -> Option<Vec<u8>> {
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
    // Confirmed state must not have changed.
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
fn test_accumulated_cache_rebuilt_after_confirmation() {
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
    assert!(read_confirmed(&s, b"k2").is_none()); // still only in cache

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
    // Establish k1=v1 in the real state via a direct confirmed update.
    s.handle_confirmed_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));

    // Preemptive seq 2: delete k1 (tombstone in cache only).
    s.handle_preemptive_update(&app, make_batch(2, &[(b"k1".to_vec(), None)]))
        .unwrap();
    // Real state still has k1.
    assert_eq!(read_confirmed(&s, b"k1"), Some(b"v1".to_vec()));

    // Confirm seq 2: tombstone is flushed → k1 gone from real state.
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

    // Backtrack to seq 2: keep seq 1, discard seq 2 and 3.
    s.backtrack(SeqNo::from(2u32)).unwrap();
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(2u32));
    assert_eq!(s.pending_count(), 1);
    assert_eq!(s.pending_front_seq(), Some(SeqNo::from(1u32)));
    assert!(read_confirmed(&s, b"k1").is_none()); // nothing confirmed
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

    // Backtrack to seq 2 — seq 1 kept, seq 2 discarded.
    s.backtrack(SeqNo::from(2u32)).unwrap();
    // Re-execute seq 2 with a corrected value.
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
    // Speculative work that will be overwritten.
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
// Fresh boot: the first batch consensus delivers is SeqNo::ZERO
// ---------------------------------------------------------------------------

#[test]
fn test_first_batch_at_seq_zero_executes() {
    let app = MapApp;
    let mut s = new_state();

    // Regression: SeqNo::ZERO is both the initial value and the first sequence number
    // consensus assigns. Tracking the last-applied seq made these indistinguishable, so
    // the very first batch of every run was classified as a backtrack and dropped.
    s.handle_preemptive_update(
        &app,
        make_batch(0, &[(b"k0".to_vec(), Some(b"v0".to_vec()))]),
    )
    .unwrap();

    assert_eq!(s.next_preemptive_seq(), SeqNo::ONE);
    assert_eq!(s.pending_front_seq(), Some(SeqNo::ZERO));

    let replies = s.handle_update_confirmed(&app, SeqNo::ZERO).unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(read_confirmed(&s, b"k0"), Some(b"v0".to_vec()));
    assert_eq!(s.next_confirmed_seq(), SeqNo::ONE);
}

// ---------------------------------------------------------------------------
// Reorder buffer
// ---------------------------------------------------------------------------

#[test]
fn test_early_batch_is_buffered_then_drained() {
    let app = MapApp;
    let mut s = new_state();

    // Seq 1 arrives before seq 0 and must be held, not dropped.
    let outcome = s
        .handle_preemptive_update(
            &app,
            make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
        )
        .unwrap();
    assert_eq!(
        outcome,
        PreemptiveOutcome::Staged {
            awaiting: SeqNo::ZERO,
            buffered: 1
        }
    );
    assert_eq!(s.pending_count(), 0);
    assert_eq!(s.staged_count(), 1);

    // Seq 0 closes the gap and releases seq 1 behind it.
    let outcome = s
        .handle_preemptive_update(
            &app,
            make_batch(0, &[(b"k0".to_vec(), Some(b"v0".to_vec()))]),
        )
        .unwrap();
    assert_eq!(outcome, PreemptiveOutcome::Executed { drained: 1 });
    assert_eq!(s.staged_count(), 0);
    assert_eq!(s.pending_count(), 2);
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(2u32));
}

#[test]
fn test_replica_zero_arrival_order_from_production_log() {
    let app = MapApp;
    let mut s = new_state();

    // The exact order in which pre-prepares completed on replica 0 in the stuck run.
    // Under the old head+1 gate this wedged permanently at seq 7.
    let arrival = [1, 2, 3, 4, 5, 6, 0, 8, 7, 9, 10];
    for seq in arrival {
        let key = format!("k{seq}").into_bytes();
        s.handle_preemptive_update(&app, make_batch(seq, &[(key, Some(b"v".to_vec()))]))
            .expect("no batch may be rejected: consensus delivers each exactly once");
    }

    // Every batch speculated, nothing left buffered.
    assert_eq!(s.staged_count(), 0);
    assert_eq!(s.pending_count(), arrival.len());
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(11u32));

    // Confirmations arrive strictly in order and every one of them lands.
    for seq in 0..arrival.len() as u32 {
        let replies = s
            .handle_update_confirmed(&app, SeqNo::from(seq))
            .unwrap_or_else(|e| panic!("confirmation for seq {seq} failed: {e:?}"));
        assert_eq!(replies.len(), 1);
    }

    assert_eq!(s.pending_count(), 0);
    for seq in arrival {
        let key = format!("k{seq}").into_bytes();
        assert_eq!(read_confirmed(&s, &key), Some(b"v".to_vec()));
    }
}

#[test]
fn test_buffered_batches_survive_a_gap_that_closes_late() {
    let app = MapApp;
    let mut s = new_state();

    // Everything from 1..=20 arrives while seq 0 is still missing.
    for seq in 1..=20u32 {
        s.handle_preemptive_update(
            &app,
            make_batch(
                seq,
                &[(format!("k{seq}").into_bytes(), Some(b"v".to_vec()))],
            ),
        )
        .unwrap();
    }
    assert_eq!(s.staged_count(), 20);
    assert_eq!(s.pending_count(), 0);

    // The straggler releases all 20 at once.
    let outcome = s
        .handle_preemptive_update(
            &app,
            make_batch(0, &[(b"k0".to_vec(), Some(b"v".to_vec()))]),
        )
        .unwrap();
    assert_eq!(outcome, PreemptiveOutcome::Executed { drained: 20 });
    assert_eq!(s.staged_count(), 0);
    assert_eq!(s.pending_count(), 21);
}

#[test]
fn test_duplicate_early_batch_does_not_double_execute() {
    let app = MapApp;
    let mut s = new_state();

    s.handle_preemptive_update(
        &app,
        make_batch(2, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]),
    )
    .unwrap();
    s.handle_preemptive_update(
        &app,
        make_batch(2, &[(b"k2".to_vec(), Some(b"v2".to_vec()))]),
    )
    .unwrap();
    assert_eq!(s.staged_count(), 1);

    s.handle_preemptive_update(
        &app,
        make_batch(0, &[(b"k0".to_vec(), Some(b"v0".to_vec()))]),
    )
    .unwrap();
    s.handle_preemptive_update(
        &app,
        make_batch(1, &[(b"k1".to_vec(), Some(b"v1".to_vec()))]),
    )
    .unwrap();

    assert_eq!(s.pending_count(), 3);
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(3u32));
}

#[test]
fn test_backtrack_drops_stale_buffered_batches() {
    let app = MapApp;
    let mut s = new_state();

    s.handle_preemptive_update(
        &app,
        make_batch(0, &[(b"k0".to_vec(), Some(b"v0".to_vec()))]),
    )
    .unwrap();
    // Seq 3 and 4 arrive early and sit in the buffer.
    for seq in [3u32, 4] {
        s.handle_preemptive_update(
            &app,
            make_batch(
                seq,
                &[(format!("k{seq}").into_bytes(), Some(b"v".to_vec()))],
            ),
        )
        .unwrap();
    }
    assert_eq!(s.staged_count(), 2);

    // A backtrack to seq 1 invalidates everything from 1 up, buffered copies included.
    s.backtrack(SeqNo::ONE).unwrap();
    assert_eq!(s.staged_count(), 0);
    assert_eq!(s.pending_count(), 1);
    assert_eq!(s.next_preemptive_seq(), SeqNo::ONE);
}

#[test]
fn test_install_state_clears_the_reorder_buffer() {
    let app = MapApp;
    let mut s = new_state();

    for seq in [5u32, 6] {
        s.handle_preemptive_update(
            &app,
            make_batch(
                seq,
                &[(format!("k{seq}").into_bytes(), Some(b"v".to_vec()))],
            ),
        )
        .unwrap();
    }
    assert_eq!(s.staged_count(), 2);

    s.install_confirmed_state(SeqNo::from(10u32), MapState::default());
    assert_eq!(s.staged_count(), 0);
    assert_eq!(s.next_preemptive_seq(), SeqNo::from(11u32));
}
