use atlas_common::collections::HashMap;
use atlas_smr_execution::crud_states::CRUDState;

/// A flat map of (column → (key → value)), where `None` is a tombstone (key deleted).
pub(super) type AccumulatedCache = HashMap<String, HashMap<Vec<u8>, Option<Vec<u8>>>>;

/// A view of state that reads from a layered cache before the real confirmed state.
/// All writes go into the local `delta` only; the real state is never touched.
pub(super) struct CachingState<'a, S> {
    confirmed_state: &'a S,
    accumulated_cache: &'a AccumulatedCache,
    pub(super) delta: AccumulatedCache,
}

impl<'a, S: CRUDState + Sync> CachingState<'a, S> {
    pub(super) fn new(confirmed_state: &'a S, accumulated_cache: &'a AccumulatedCache) -> Self {
        Self {
            confirmed_state,
            accumulated_cache,
            delta: AccumulatedCache::default(),
        }
    }

    pub(super) fn into_delta(self) -> AccumulatedCache {
        self.delta
    }
}

impl<'a, S: CRUDState + Sync> CRUDState for CachingState<'a, S> {
    fn read(&self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
        // Priority: local delta → accumulated cache → real confirmed state.
        // A `None` entry in any cache layer is a tombstone (key deleted).
        if let Some(col) = self.delta.get(column)
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

    fn create(&mut self, column: &str, key: &[u8], value: &[u8]) -> bool {
        // Key must not exist (a tombstone counts as non-existent).
        if self.read(column, key).is_some() {
            return false;
        }
        self.delta
            .entry(column.to_string())
            .or_default()
            .insert(key.to_vec(), Some(value.to_vec()));
        true
    }

    fn update(&mut self, column: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>> {
        let old = self.read(column, key);
        self.delta
            .entry(column.to_string())
            .or_default()
            .insert(key.to_vec(), Some(value.to_vec()));
        old
    }

    fn delete(&mut self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
        let old = self.read(column, key);
        // Insert tombstone so subsequent reads in this batch see the key as deleted.
        self.delta
            .entry(column.to_string())
            .or_default()
            .insert(key.to_vec(), None);
        old
    }
}

/// Merge all entries from `src` into `dst`, with `src` overriding `dst` for the same key.
pub(super) fn merge_delta_into(dst: &mut AccumulatedCache, src: &AccumulatedCache) {
    for (col, keys) in src {
        let col_map = dst.entry(col.clone()).or_default();
        for (key, val) in keys {
            col_map.insert(key.clone(), val.clone());
        }
    }
}

/// Apply a delta to a real CRUD state: `Some(v)` → update, `None` → delete.
pub(super) fn apply_delta_to_state<S: CRUDState>(state: &mut S, delta: &AccumulatedCache) {
    for (col, keys) in delta {
        for (key, val) in keys {
            match val {
                Some(v) => {
                    state.update(col, key, v);
                }
                None => {
                    state.delete(col, key);
                }
            }
        }
    }
}

/// Rebuild an accumulated cache by merging deltas in order (later deltas override earlier ones).
pub(super) fn rebuild_accumulated_cache<'a>(
    deltas: impl Iterator<Item = &'a AccumulatedCache>,
) -> AccumulatedCache {
    let mut acc = AccumulatedCache::default();
    for delta in deltas {
        merge_delta_into(&mut acc, delta);
    }
    acc
}
