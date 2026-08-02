use atlas_common::collections::HashMap;
use atlas_smr_execution::crud_states::{Access, AccessType, CRUDState};
use std::cell::UnsafeCell;
use std::collections::BTreeSet;

use crate::single_threaded_crud::caching_state::AccumulatedCache;

// ---------------------------------------------------------------------------
// ParallelExecutionUnit
// ---------------------------------------------------------------------------

/// Per-request proxy state for the parallel speculative phase.
///
/// Reads from: own local cache → upper accumulated cache → confirmed state.
/// All reads (including fallthrough) are recorded as accesses for collision detection.
/// Writes and deletes go only into the local cache; the upper layers are never mutated.
pub(super) struct ParallelExecutionUnit<'a, S> {
    confirmed_state: &'a S,
    accumulated_cache: &'a AccumulatedCache,
    /// Tombstone-aware local cache: `None` = deleted.
    local_cache: HashMap<String, HashMap<Vec<u8>, Option<Vec<u8>>>>,
    /// Every read/write/delete on any key is appended here.
    /// `UnsafeCell` allows appending from `read(&self, ...)` without a mutable receiver.
    /// Safety invariant: only one thread accesses this at a time (each unit is per-request).
    accesses: UnsafeCell<Vec<Access>>,
}

// SAFETY: `&'a S: Send` when `S: Sync`. `AccumulatedCache: Sync`. `UnsafeCell<Vec<Access>>:
// Send` because `Vec<Access>: Send`. Each unit is accessed from exactly one thread at a time.
unsafe impl<'a, S: CRUDState + Sync> Send for ParallelExecutionUnit<'a, S> {}

impl<'a, S: CRUDState + Sync> ParallelExecutionUnit<'a, S> {
    pub(super) fn new(confirmed_state: &'a S, accumulated_cache: &'a AccumulatedCache) -> Self {
        Self {
            confirmed_state,
            accumulated_cache,
            local_cache: HashMap::default(),
            accesses: UnsafeCell::new(Vec::new()),
        }
    }

    /// Consume the unit and return the (access log, local cache delta).
    pub(super) fn complete(self) -> (Vec<Access>, AccumulatedCache) {
        (self.accesses.into_inner(), self.local_cache)
    }

    /// Three-tier read without recording an access (used internally by write methods).
    fn read_raw(&self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
        if let Some(col) = self.local_cache.get(column)
            && let Some(val) = col.get(key)
        {
            return val.clone();
        }
        if let Some(col) = self.accumulated_cache.get(column)
            && let Some(val) = col.get(key)
        {
            return val.clone();
        }
        self.confirmed_state.read(column, key)
    }
}

impl<'a, S: CRUDState + Sync> CRUDState for ParallelExecutionUnit<'a, S> {
    fn read(&self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
        // SAFETY: `accesses` is only accessed from the one thread that owns this unit.
        // No aliasing occurs: `read` and the mutable methods are never called concurrently
        // on the same unit, and the unit is never shared across threads.
        unsafe { &mut *self.accesses.get() }.push(Access::new(
            column,
            key.to_vec(),
            AccessType::Read,
        ));
        self.read_raw(column, key)
    }

    fn create(&mut self, column: &str, key: &[u8], value: &[u8]) -> bool {
        if self.read_raw(column, key).is_some() {
            return false;
        }
        self.accesses
            .get_mut()
            .push(Access::new(column, key.to_vec(), AccessType::Write));
        self.local_cache
            .entry(column.to_string())
            .or_default()
            .insert(key.to_vec(), Some(value.to_vec()));
        true
    }

    fn update(&mut self, column: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>> {
        let old = self.read_raw(column, key);
        self.accesses
            .get_mut()
            .push(Access::new(column, key.to_vec(), AccessType::Write));
        self.local_cache
            .entry(column.to_string())
            .or_default()
            .insert(key.to_vec(), Some(value.to_vec()));
        old
    }

    fn delete(&mut self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
        let old = self.read_raw(column, key);
        self.accesses
            .get_mut()
            .push(Access::new(column, key.to_vec(), AccessType::Delete));
        self.local_cache
            .entry(column.to_string())
            .or_default()
            .insert(key.to_vec(), None);
        old
    }
}

// ---------------------------------------------------------------------------
// CollisionState
// ---------------------------------------------------------------------------

/// Per-key access record: set of access types seen + set of batch positions that accessed it.
type AccessRecord = (BTreeSet<AccessType>, BTreeSet<usize>);

/// Tracks which `(column, key)` pairs have been accessed (and by which batch positions),
/// detecting conflicts between parallel operations in the same batch.
///
/// Collision rules (same as `atlas-smr-execution`):
/// - Write + anything → collision
/// - Delete + anything → collision
/// - Read + Read → no collision
///
/// Uses `(column, key)` as the tracking key (more precise than the scalable SMR executor,
/// which only uses `key`).
#[derive(Default)]
pub(super) struct CollisionState {
    accessed: HashMap<(String, Vec<u8>), AccessRecord>,
    pub(super) collisions: BTreeSet<usize>,
}

/// Update `state` with the accesses from a single execution unit at `position`.
/// Returns `true` if this unit collided with any previously registered unit.
pub(super) fn progress_collision_state(
    state: &mut CollisionState,
    position: usize,
    accesses: &[Access],
) -> bool {
    let mut collided = false;

    for access in accesses {
        let key = (access.column().clone(), access.key().clone());
        let entry = state.accessed.entry(key).or_default();
        let (seen_types, seen_positions) = entry;

        // Check against all access types already registered for this key.
        for seen_type in seen_types.iter() {
            if seen_type.is_collision(&access.access_type()) {
                // Both the current operation and all prior accessors collide.
                state.collisions.insert(position);
                state.collisions.extend(seen_positions.iter().copied());
                collided = true;
                break;
            }
        }

        seen_types.insert(access.access_type());
        seen_positions.insert(position);
    }

    collided
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_smr_execution::crud_states::CRUDState;

    /// Minimal in-memory CRUD state for testing.
    #[derive(Default, Clone)]
    struct TestState(HashMap<String, HashMap<Vec<u8>, Vec<u8>>>);

    impl CRUDState for TestState {
        fn read(&self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
            self.0.get(column)?.get(key).cloned()
        }
        fn create(&mut self, column: &str, key: &[u8], value: &[u8]) -> bool {
            let col = self.0.entry(column.to_string()).or_default();
            if col.contains_key(key) {
                return false;
            }
            col.insert(key.to_vec(), value.to_vec());
            true
        }
        fn update(&mut self, column: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>> {
            self.0
                .entry(column.to_string())
                .or_default()
                .insert(key.to_vec(), value.to_vec())
        }
        fn delete(&mut self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
            self.0.get_mut(column)?.remove(key)
        }
    }

    fn empty_cache() -> AccumulatedCache {
        AccumulatedCache::default()
    }

    // ---------------------------------------------------------------------------
    // Read priority tests
    // ---------------------------------------------------------------------------

    #[test]
    fn read_from_confirmed_when_caches_empty() {
        let mut state = TestState::default();
        state.update("c", b"k", b"v_conf");
        let cache = empty_cache();
        let unit = ParallelExecutionUnit::new(&state, &cache);
        assert_eq!(unit.read("c", b"k"), Some(b"v_conf".to_vec()));
    }

    #[test]
    fn read_from_accumulated_cache_overrides_confirmed() {
        let mut state = TestState::default();
        state.update("c", b"k", b"v_conf");
        let mut cache = empty_cache();
        cache
            .entry("c".to_string())
            .or_default()
            .insert(b"k".to_vec(), Some(b"v_acc".to_vec()));
        let unit = ParallelExecutionUnit::new(&state, &cache);
        assert_eq!(unit.read("c", b"k"), Some(b"v_acc".to_vec()));
    }

    #[test]
    fn read_from_local_cache_overrides_all() {
        let mut state = TestState::default();
        state.update("c", b"k", b"v_conf");
        let mut cache = empty_cache();
        cache
            .entry("c".to_string())
            .or_default()
            .insert(b"k".to_vec(), Some(b"v_acc".to_vec()));
        let mut unit = ParallelExecutionUnit::new(&state, &cache);
        unit.update("c", b"k", b"v_local");
        assert_eq!(unit.read("c", b"k"), Some(b"v_local".to_vec()));
    }

    #[test]
    fn tombstone_in_local_cache_hides_key() {
        let mut state = TestState::default();
        state.update("c", b"k", b"v");
        let cache = empty_cache();
        let mut unit = ParallelExecutionUnit::new(&state, &cache);
        unit.delete("c", b"k");
        assert_eq!(unit.read("c", b"k"), None);
    }

    #[test]
    fn tombstone_in_accumulated_cache_hides_confirmed() {
        let mut state = TestState::default();
        state.update("c", b"k", b"v");
        let mut cache = empty_cache();
        cache
            .entry("c".to_string())
            .or_default()
            .insert(b"k".to_vec(), None); // tombstone
        let unit = ParallelExecutionUnit::new(&state, &cache);
        assert_eq!(unit.read("c", b"k"), None);
    }

    #[test]
    fn read_records_access() {
        let state = TestState::default();
        let cache = empty_cache();
        let unit = ParallelExecutionUnit::new(&state, &cache);
        unit.read("c", b"k");
        let (accesses, _) = unit.complete();
        assert_eq!(accesses.len(), 1);
        assert_eq!(accesses[0].access_type(), AccessType::Read);
    }

    #[test]
    fn write_records_access() {
        let state = TestState::default();
        let cache = empty_cache();
        let mut unit = ParallelExecutionUnit::new(&state, &cache);
        unit.update("c", b"k", b"v");
        let (accesses, _) = unit.complete();
        assert_eq!(accesses.len(), 1);
        assert_eq!(accesses[0].access_type(), AccessType::Write);
    }

    #[test]
    fn delete_records_access() {
        let state = TestState::default();
        let cache = empty_cache();
        let mut unit = ParallelExecutionUnit::new(&state, &cache);
        unit.delete("c", b"k");
        let (accesses, _) = unit.complete();
        assert_eq!(accesses.len(), 1);
        assert_eq!(accesses[0].access_type(), AccessType::Delete);
    }

    // ---------------------------------------------------------------------------
    // CollisionState tests
    // ---------------------------------------------------------------------------

    #[test]
    fn read_read_no_collision() {
        let mut cs = CollisionState::default();
        let a0 = [Access::new("c", b"k".to_vec(), AccessType::Read)];
        let a1 = [Access::new("c", b"k".to_vec(), AccessType::Read)];
        assert!(!progress_collision_state(&mut cs, 0, &a0));
        assert!(!progress_collision_state(&mut cs, 1, &a1));
        assert!(cs.collisions.is_empty());
    }

    #[test]
    fn write_write_collision() {
        let mut cs = CollisionState::default();
        let a0 = [Access::new("c", b"k".to_vec(), AccessType::Write)];
        let a1 = [Access::new("c", b"k".to_vec(), AccessType::Write)];
        progress_collision_state(&mut cs, 0, &a0);
        progress_collision_state(&mut cs, 1, &a1);
        assert!(cs.collisions.contains(&0));
        assert!(cs.collisions.contains(&1));
    }

    #[test]
    fn write_read_collision() {
        let mut cs = CollisionState::default();
        let a0 = [Access::new("c", b"k".to_vec(), AccessType::Write)];
        let a1 = [Access::new("c", b"k".to_vec(), AccessType::Read)];
        progress_collision_state(&mut cs, 0, &a0);
        progress_collision_state(&mut cs, 1, &a1);
        assert!(cs.collisions.contains(&0));
        assert!(cs.collisions.contains(&1));
    }

    #[test]
    fn delete_anything_collision() {
        let mut cs = CollisionState::default();
        let a0 = [Access::new("c", b"k".to_vec(), AccessType::Delete)];
        let a1 = [Access::new("c", b"k".to_vec(), AccessType::Read)];
        progress_collision_state(&mut cs, 0, &a0);
        progress_collision_state(&mut cs, 1, &a1);
        assert!(cs.collisions.contains(&0));
        assert!(cs.collisions.contains(&1));
    }

    #[test]
    fn different_keys_no_collision() {
        let mut cs = CollisionState::default();
        let a0 = [Access::new("c", b"k1".to_vec(), AccessType::Write)];
        let a1 = [Access::new("c", b"k2".to_vec(), AccessType::Write)];
        progress_collision_state(&mut cs, 0, &a0);
        progress_collision_state(&mut cs, 1, &a1);
        assert!(cs.collisions.is_empty());
    }

    #[test]
    fn different_columns_same_key_no_collision() {
        // (column, key) is the tracking unit — same key in different columns don't collide.
        let mut cs = CollisionState::default();
        let a0 = [Access::new("col1", b"k".to_vec(), AccessType::Write)];
        let a1 = [Access::new("col2", b"k".to_vec(), AccessType::Write)];
        progress_collision_state(&mut cs, 0, &a0);
        progress_collision_state(&mut cs, 1, &a1);
        assert!(cs.collisions.is_empty());
    }
}
