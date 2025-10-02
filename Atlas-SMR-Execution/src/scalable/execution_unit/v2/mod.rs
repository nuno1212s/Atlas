use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use getset::CopyGetters;
use atlas_common::collections::HashMap;
use atlas_common::ordering::SeqNo;
use crate::scalable::{Access, AccessType, CRUDState};

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
    seq_no: SeqNo,
    /// The position of this request within the general batch
    affected_positions: BTreeSet<usize>,
    /// The data accesses this request wants to perform
    accesses: RefCell<BTreeMap<usize, Vec<Access>>>,
    /// A cache containing all of the overwritten values by this operation,
    /// So we don't read stale values from the state.
    cache: HashMap<String, HashMap<Vec<u8>, Vec<u8>>>,
    /// State reference a reference to the state
    state_reference: &'a S,
    currently_executing: Option<usize>,
}

/// # Safety
/// We can safely do this since the execution units
//  are only used within a single thread and are not shared between threads
/// This is necessary since we have to resort to a [RefCell] for interior
/// mutability with the read operation.
unsafe impl<'a, S> Sync for crate::scalable::execution_unit::ExecutionUnit<'a, S> where S: CRUDState {}

unsafe impl<'a, S> Send for crate::scalable::execution_unit::ExecutionUnit<'a, S> where S: CRUDState {}

impl<'a, S> CRUDState for crate::scalable::execution_unit::ExecutionUnit<'a, S>
    where
        S: CRUDState,
{
    fn create(&mut self, column: &str, key: &[u8], value: &[u8]) -> bool {
        self.accesses
            .borrow_mut()
            .entry(self.currently_executing.clone().unwrap())
            .or_insert_with(Vec::default)
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
            .entry(self.currently_executing.clone().unwrap())
            .or_insert_with(Vec::default)
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
            .entry(self.currently_executing.clone().unwrap())
            .or_insert_with(Vec::default)
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
            .entry(self.currently_executing.clone().unwrap())
            .or_insert_with(Vec::default)
            .push(Access::init(column, key.to_vec(), AccessType::Delete));

        if self.cache.contains_key(column) {
            let column = self.cache.get_mut(column).unwrap();

            column.remove(key)
        } else {
            None
        }
    }
}

impl<'a, S> crate::scalable::execution_unit::ExecutionUnit<'a, S>
    where
        S: CRUDState,
{
    pub fn new(seq_no: SeqNo, state: &'a S) -> Self {
        Self {
            seq_no,
            affected_positions: Default::default(),
            accesses: RefCell::new(Default::default()),
            cache: Default::default(),
            state_reference: state,
            currently_executing: None,
        }
    }

    pub fn start_execution_of(&mut self, operation: usize) {
        self.currently_executing = Some(operation);
    }

    pub fn done_execution_of_op(&mut self) {
        let executed = self.currently_executing.take();

        if let Some(executed) = executed {
            self.affected_positions.insert(executed);
        }
    }

    ///
    pub fn complete(self) -> crate::scalable::execution_unit::ExecutionResult {
        crate::scalable::execution_unit::ExecutionResult {
            seq_no: self.seq_no,
            affected_positions: self.affected_positions,
            alterations: self.accesses.into_inner(),
            cache: self.cache,
        }
    }
}

#[derive(CopyGetters)]
/// Result of the execution of an execution unit
pub struct ExecutionResult {
    /// The sequence number of the batch this operation belongs to
    seq_no: SeqNo,
    /// The position of this request within the general batch
    #[get_copy]
    affected_positions: BTreeSet<usize>,
    /// The data accesses this request wants to perform
    alterations: BTreeMap<usize, Vec<Access>>,
    /// A cache containing all of the overwritten values by this operation,
    cache: HashMap<String, HashMap<Vec<u8>, Vec<u8>>>,
}
