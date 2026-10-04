# Prompt: stop rebuilding the accumulated cache on confirm

Paste this into a session opened on the Atlas repository (branch `preemptive_execution`).

---

In `Atlas-SMR-Preemptive-Execution`, both cache executors rebuild the accumulated cache on every
confirmation. Only a backtrack needs that. Change both executors so that a confirmation leaves the
cache as it is.

**Where the rebuild happens today**
- `src/single_threaded_crud/pending_state.rs`, `handle_update_confirmed`: after
  `apply_delta_to_state(&mut self.confirmed_state, &update.delta)` (around line 312), it calls
  `self.accumulated_cache = rebuild_accumulated_cache(self.pending.iter().map(|p| &p.delta));`
  (around line 316).
- `src/scalable_crud/pending_state.rs`, `handle_update_confirmed`: the same pair, around lines
  389–390.

**Why the rebuild can go.** After the head's delta is applied to the confirmed state, every entry
the cache still holds from that delta is in one of two cases:
- it equals the confirmed state's value for that key; or
- a newer pending delta has already overridden it in the cache.

Reads resolve in the order local delta → cache → confirmed state, so they return the same value
with or without the rebuild. Tombstones behave the same way: a confirmed delete leaves ⊥ in the
cache and removes the key from the confirmed state, and both read as "absent".

**What to change**
1. In both `handle_update_confirmed` functions, keep the `apply_delta_to_state` call and drop the
   `rebuild_accumulated_cache` call on the speculated (`ConfirmRoute::Pending`) path.
2. When the confirmation empties `pending`, clear the cache
   (`self.accumulated_cache.clear()`). Otherwise it would grow without bound, holding only
   confirmed entries.
3. Guard every path that writes the confirmed state **without** going through a pending delta. If
   the cache still held confirmed entries there, they would shadow the newer value, and a later
   read would return stale data.
   - `handle_confirmed_update` (the `UpdateBatch` path, which only checks that `pending` is empty):
     clear the cache before executing the batch directly.
   - The `ConfirmRoute::Staged` fallback, which calls `discard_speculation()` and then
     `execute_directly`: check that `discard_speculation` leaves the cache empty, and that
     `execute_directly` never runs over a non-empty cache. If either can happen, clear the cache.
   - `CatchUp`, `install_confirmed_state` and state transfer: confirm they already clear the
     cache.
4. Keep the rebuild in `backtrack` and in `discard_speculation`. Those paths drop entries, so they
   need it.
5. Keep the metrics meaning the same. `SCALABLE_CONFIRM_APPLICATION_TIME` and the CRUD cache's
   confirm-apply timing now measure only the apply, so say so in the comment next to them.

**Tests**
- Update any unit test that asserts the cache equals the fold of the remaining pending deltas right
  after a confirm. The new invariant is weaker: for every key, reading through cache → confirmed
  state gives the same value as reading through "fold of pending" → confirmed state.
- Add a test for that read-equivalence invariant after a sequence of speculate and confirm steps,
  including deletes.
- Add a test for the hazard in step 3:
  1. speculate and confirm a write to key `k`, so `pending` drains and the cache is cleared;
  2. then run an `UpdateBatch` that writes `k` again directly;
  3. then speculate a batch that reads `k`. It must see the direct write.
- Run `cargo test -p atlas-smr-preemptive-execution` and
  `cargo clippy -p atlas-smr-preemptive-execution --no-deps -- -D warnings`.

**Docs**
- Update `docs/02-crud-cache-executor.md` and `docs/03-scalable-crud-executor.md` wherever they say
  that a confirmation rebuilds the cache.
- Finding 9 in `docs/context/crud-cache-visuals.md` describes this change. Mark it as resolved once
  the code matches.
