//mod v2;

use crate::scalable::{Access, AccessType, CRUDState};
use atlas_common::collections::HashMap;
use atlas_common::ordering::SeqNo;
use getset::{CopyGetters, Getters};
use std::cell::RefCell;
use std::collections::BTreeSet;

/// A data structure that represents a single execution unit
/// Which can be speculatively parallelized.
/// This can be interpreted as a state since it implements CRUDState.
/// All changes made here are done in a local cache and are only applied to
/// the state once it is verified to be free of collisions from other operations.
pub struct ExecutionUnit<'a, S>
where
    S: CRUDState,
{
    /// The sequence number of the batch this operation belongs to
    pub(super) seq_no: SeqNo,
    /// The position of this request within the general batch
    pub(super) position_in_batch: usize,
    /// The data accesses this request wants to perform
    pub(super) accesses: RefCell<Vec<Access>>,
    /// A cache containing all of the overwritten values by this operation,
    /// So we don't read stale values from the state.
    pub(super) cache: HashMap<String, HashMap<Vec<u8>, Vec<u8>>>,
    /// State reference a reference to the state
    pub(super) state_reference: &'a S,
}

/// # Safety
/// We can safely do this since the execution units
//  are only used within a single thread and are not shared between threads
/// This is necessary since we have to resort to a [RefCell] for interior
/// mutability with the read operation.
unsafe impl<'a, S> Sync for ExecutionUnit<'a, S> where S: CRUDState {}

unsafe impl<'a, S> Send for ExecutionUnit<'a, S> where S: CRUDState {}

impl<'a, S> CRUDState for ExecutionUnit<'a, S>
where
    S: CRUDState,
{
    fn create(&mut self, column: &str, key: &[u8], value: &[u8]) -> bool {
        self.accesses
            .borrow_mut()
            .push(Access::init(column, key.to_vec(), AccessType::Write));

        if self.cache.contains_key(column) {
            let column = self.cache.get_mut(column).unwrap();

            if column.contains_key(key) {
                false
            } else {
                column.insert(key.to_vec(), value.to_vec());
                true
            }
        } else {
            let mut column_map = HashMap::default();

            column_map.insert(key.to_vec(), value.to_vec());

            self.cache.insert(column.to_string(), column_map);

            true
        }
    }

    fn read(&self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
        self.accesses
            .borrow_mut()
            .push(Access::init(column, key.to_vec(), AccessType::Read));

        if self.cache.contains_key(column) {
            let column = self.cache.get(column).unwrap();

            column.get(key).cloned()
        } else {
            self.state_reference.read(column, key)
        }
    }

    fn update(&mut self, column: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>> {
        self.accesses
            .borrow_mut()
            .push(Access::init(column, key.to_vec(), AccessType::Write));

        if self.cache.contains_key(column) {
            let column = self.cache.get_mut(column).unwrap();

            column.insert(key.to_vec(), value.to_vec())
        } else {
            let mut column_map = HashMap::default();

            column_map.insert(key.to_vec(), value.to_vec());

            self.cache.insert(column.to_string(), column_map);

            None
        }
    }

    fn delete(&mut self, column: &str, key: &[u8]) -> Option<Vec<u8>> {
        self.accesses
            .borrow_mut()
            .push(Access::init(column, key.to_vec(), AccessType::Delete));

        if self.cache.contains_key(column) {
            let column = self.cache.get_mut(column).unwrap();

            column.remove(key)
        } else {
            None
        }
    }
}

impl<'a, S> ExecutionUnit<'a, S>
where
    S: CRUDState,
{
    pub fn complete(self) -> ExecutionResult {
        ExecutionResult {
            seq_no: self.seq_no,
            position_in_batch: self.position_in_batch,
            alterations: self.accesses.into_inner(),
            cache: self.cache,
        }
    }
}

#[derive(CopyGetters, Getters)]
/// Result of the execution of an execution unit
pub struct ExecutionResult {
    /// The sequence number of the batch this operation belongs to
    seq_no: SeqNo,
    /// The position of this request within the general batch
    #[get_copy = "pub(super)"]
    position_in_batch: usize,
    /// The data accesses this request wants to perform
    #[get = "pub(super)"]
    alterations: Vec<Access>,
    /// A cache containing all of the overwritten values by this operation,
    #[get = "pub(super)"]
    cache: HashMap<String, HashMap<Vec<u8>, Vec<u8>>>,
}

pub struct Collisions {
    /// The indexes within the batch of the operations
    collided: BTreeSet<usize>,
}

#[derive(Default)]
pub(super) struct CollisionState {
    accessed: HashMap<Vec<u8>, (BTreeSet<AccessType>, BTreeSet<usize>)>,
    pub(super) collisions: BTreeSet<usize>,
}

/// Progress the collision state with the given execution unit
/// This allows the thread to not be waiting the finish of all tasks before actually starting to
/// calculate the collisions
pub(super) fn progress_collision_state(state: &mut CollisionState, unit: &ExecutionResult) -> bool {
    let mut collided = false;

    unit.alterations.iter().for_each(|access| {
        if let Some((accesses, operation_seq)) = state.accessed.get_mut(&access.key) {
            for access_type in accesses.iter() {
                if access_type.is_collision(&access.access_type) {
                    state.collisions.insert(unit.position_in_batch);

                    for seq in operation_seq.iter() {
                        state.collisions.insert(*seq);
                    }

                    collided = true;
                }
            }

            operation_seq.insert(unit.position_in_batch);
            accesses.insert(access.access_type);
        } else {
            let mut accesses = BTreeSet::new();
            let mut accessors = BTreeSet::new();

            accessors.insert(unit.position_in_batch);
            accesses.insert(access.access_type);

            state
                .accessed
                .insert(access.key.clone(), (accesses, accessors));
        }
    });

    collided
}

type InnerCollisions = HashMap<Vec<u8>, (Vec<AccessType>, Vec<usize>)>;

/// Calculate collisions of data accesses within a batch, returns all
/// of the operations that must be executed sequentially
fn calculate_collisions<S>(execution_units: Vec<ExecutionUnit<S>>) -> Option<Collisions>
where
    S: CRUDState,
{
    let mut accessed: HashMap<String, InnerCollisions> = Default::default();

    let mut collisions = BTreeSet::new();

    for unit in execution_units {
        unit.accesses.borrow().iter().for_each(|access| {
            if let Some(accesses) = accessed.get_mut(&access.column) {
                if let Some((accesses, operation_seq)) = accesses.get_mut(&access.key) {
                    for access_type in accesses.iter() {
                        if access_type.is_collision(&access.access_type) {
                            collisions.insert(unit.position_in_batch);

                            for seq in operation_seq.iter() {
                                collisions.insert(*seq);
                            }
                        }
                    }

                    operation_seq.push(unit.position_in_batch);
                    accesses.push(access.access_type);
                } else {
                    accesses.insert(
                        access.key.clone(),
                        (vec![access.access_type], vec![unit.position_in_batch]),
                    );
                }
            } else {
                let mut accesses: HashMap<Vec<u8>, (Vec<AccessType>, Vec<usize>)> =
                    Default::default();

                accesses.insert(
                    access.key.clone(),
                    (vec![access.access_type], vec![unit.position_in_batch]),
                );

                accessed.insert(access.column.clone(), accesses);
            }
        });
    }

    if collisions.is_empty() {
        None
    } else {
        Some(Collisions {
            collided: collisions,
        })
    }
}
