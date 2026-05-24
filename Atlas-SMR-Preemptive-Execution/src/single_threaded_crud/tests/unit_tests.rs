//! Unit tests for `CachingPreemptiveState`.
//!
//! These tests exercise the state-machine logic directly — no channels, no threads.

use atlas_common::ordering::SeqNo;
use atlas_smr_execution::crud_states::CRUDState;

use super::test_fixtures::{MapApp, MapState, make_batch};
use crate::single_threaded_crud::pending_state::{BacktrackError, CachingPreemptiveState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn new_state() -> CachingPreemptiveState<MapState, MapApp> {
    CachingPreemptiveState::new((SeqNo::ZERO, MapState::default()))
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
    // Confirmed state must not have changed.
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
    assert!(read_confirmed(&s, b"k2").is_none()); // still only in cache

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

    // Backtrack to seq 2: keep seq 1, discard seq 2 and 3.
    s.backtrack(SeqNo::from(2u32)).unwrap();
    assert_eq!(s.preemptive_seq_no(), SeqNo::from(1u32));
    assert_eq!(s.pending_count(), 1);
    assert_eq!(s.pending_front_seq(), Some(SeqNo::from(1u32)));
    assert!(read_confirmed(&s, b"k1").is_none()); // nothing confirmed
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

    // Backtrack to seq 2 — seq 1 kept, seq 2 discarded.
    s.backtrack(SeqNo::from(2u32)).unwrap();
    // Re-execute seq 2 with a corrected value.
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
