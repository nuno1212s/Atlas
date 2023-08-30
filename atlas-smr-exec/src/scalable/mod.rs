pub mod divisible_state_exec;
pub mod monolithic_exec;

use std::cell::RefCell;
use atlas_common::collections::HashMap;
use atlas_common::ordering::SeqNo;
use atlas_execution::app::Application;
use atlas_execution::serialize::ApplicationData;

/// Types of accesses to data stored in the state
enum Access {
    Read(Vec<u8>),
    Write(Vec<u8>),
}

/// A trait defining the CRUD operations required to be implemented for a given state in order
/// for it to be utilized as a scalable state.
pub trait CRUDState {

    /// Create a new entry in the state
    fn create(&mut self, key: &[u8], value: &[u8]) -> bool;

    /// Read an entry from the state
    fn read(&self, key: &[u8]) -> Option<Vec<u8>>;

    /// Update an entry in the state
    /// Returns the previous value that was stored in the state
    fn update(&mut self, key: &[u8], value: &[u8]) -> Option<Vec<u8>>;

    /// Delete an entry in the state
    fn delete(&mut self, key: &[u8]) -> Option<Vec<u8>>;
}

/// A data structure that represents a single execution unit
/// Which can be speculatively parallelized
pub struct ExecutionUnit<'a, S> where S: CRUDState {
    /// The sequence number of the batch this operation belongs to
    seq_no: SeqNo,
    /// The position of this request within the general batch
    position_in_batch: usize,
    /// The data accesses this request wants to perform
    alterations: RefCell<Vec<Access>>,
    /// A cache containing all of the overwritten values by this operation,
    /// So we don't read stale values from the state.
    cache: HashMap<Vec<u8>, Vec<u8>>,
    /// State reference a reference to the state
    state_reference: &'a S
}

impl<S> CRUDState for ExecutionUnit<S> where S: CRUDState {

    fn create(&mut self, key: &[u8], value: &[u8]) -> bool {
        self.alterations.borrow_mut().push(Access::Write(key.to_vec()));

        if self.cache.contains_key(key) {
            false
        } else if let Some(value) =  self.state_reference.read(key) {
            self.cache.insert(key.to_vec(), value);

            false
        } else {
            self.cache.insert(key.to_vec(), value.to_vec());
            true
        }
    }

    fn read(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.alterations.borrow_mut().push(Access::Read(key.to_vec()));

        if self.cache.contains_key(key) {
            self.cache.get(key).cloned()
        } else {
            self.state_reference.read(key)
        }
    }

    fn update(&mut self, key: &[u8], value: &Vec<u8>) -> Option<Vec<u8>> {
        self.alterations.borrow_mut().push(Access::Write(key.to_vec()));

        self.cache.insert(key.to_vec(), value.to_vec())
    }

    fn delete(&mut self, key: &Vec<u8>) -> Option<Vec<u8>> {
        self.alterations.borrow_mut().push(Access::Write(key.to_vec()));

        self.cache.remove(key)
    }
}

/// Speculatively attempt to execute a given request.
/// This creates an execution unit, which stores all of the data accesses performed by the request along with
/// A cache, both for read values and for altered values.
fn speculative_execution<A, S>(application: &A, seq_no: SeqNo, pos_in_batch: usize, state: &S, request: A::AppData::Request) -> (ExecutionUnit<S>, A::AppData::Reply)
    where A: Application<S>, S: CRUDState {

    let mut exec_unit = ExecutionUnit {
        seq_no,
        position_in_batch: pos_in_batch,
        alterations: Default::default(),
        cache: Default::default(),
        state_reference: state,
    };

    let reply = application.update(&mut exec_unit, request);

    (exec_unit, reply)
}
