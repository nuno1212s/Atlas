#![allow(dead_code)]
use std::ops::{Deref, DerefMut};

use atlas_common::ordering::SeqNo;

/// A structure that holds two copies of a state for preemptive execution.
///
/// The `preemptive_state_copy` is used to execute operations that are preemptively
/// executed, while the main state is used for regular execution.
pub(super) struct DuplicateState<S> {
    // The state copy used for preemptive execution.
    preemptive_state_copy: S,
    current_confirmed_seq_no: SeqNo,
    // The main state copy used for confirmed execution used as a source of truth.
    confirmed_state_copy: S,
}

impl<S: Clone> DuplicateState<S> {
    /// Creates a new `DuplicateState` with both state copies initialized to the same value.
    ///
    /// # Arguments
    ///
    /// * `initial_state` - The initial state to be cloned for both copies.
    pub fn new(initial_state: S) -> Self {
        Self {
            preemptive_state_copy: initial_state.clone(),
            current_confirmed_seq_no: SeqNo::ZERO,
            confirmed_state_copy: initial_state,
        }
    }

    /// Gets a mutable reference to the preemptive state copy.
    fn preemptive_state(&mut self) -> &mut S {
        &mut self.preemptive_state_copy
    }

    /// Gets a mutable reference to the confirmed state copy.
    pub fn confirmed_state(&mut self) -> ConfirmedStateGuard<'_, S> {
        ConfirmedStateGuard::new(self)
    }

    /// Synchronizes the preemptive state copy with the confirmed state copy.
    fn sync_preemptive_with_confirmed(&mut self) {
        self.preemptive_state_copy = self.confirmed_state_copy.clone();
    }
}

pub(super) struct ConfirmedStateGuard<'a, S>
where
    S: Clone,
{
    state: &'a mut DuplicateState<S>,
}

impl<'a, S> ConfirmedStateGuard<'a, S>
where
    S: Clone,
{
    pub fn new(state: &'a mut DuplicateState<S>) -> Self {
        Self { state }
    }
}

impl<'a, S> Deref for ConfirmedStateGuard<'a, S>
where
    S: Clone,
{
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.state.confirmed_state_copy
    }
}

impl<'a, S> DerefMut for ConfirmedStateGuard<'a, S>
where
    S: Clone,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state.confirmed_state_copy
    }
}

impl<'a, S> Drop for ConfirmedStateGuard<'a, S>
where
    S: Clone,
{
    fn drop(&mut self) {
        self.state.sync_preemptive_with_confirmed();
    }
}
