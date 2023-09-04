pub mod divisible_state_exec;
pub mod monolithic_exec;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use scoped_threadpool::Pool;
use atlas_common::channel;
use atlas_common::collections::HashMap;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_execution::app::{Application, BatchReplies, Reply, UnorderedBatch, UpdateBatch, UpdateReply};
use atlas_execution::serialize::ApplicationData;

/// How many threads should we use in the execution threadpool
const THREAD_POOL_THREADS: u32 = 4;

struct Access {
    key: Vec<u8>,
    access_type: AccessType,
}

/// Types of accesses to data stored in the state
#[derive(Copy, Clone, Debug, PartialOrd, PartialEq, Eq, Ord)]
enum AccessType {
    Read,
    Write,
    Delete,
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
/// Which can be speculatively parallelized.
/// This can be interpreted as a state since it implements CRUDState.
/// All changes made here are done in a local cache and are only applied to
/// the state once it is verified to be free of collisions from other operations.
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
    state_reference: &'a S,
}

impl<S> CRUDState for ExecutionUnit<S> where S: CRUDState {
    fn create(&mut self, key: &[u8], value: &[u8]) -> bool {
        self.alterations.borrow_mut().push(Access::init(key.to_vec(), AccessType::Write));

        if self.cache.contains_key(key) {
            false
        } else if let Some(value) = self.state_reference.read(key) {
            self.cache.insert(key.to_vec(), value);

            false
        } else {
            self.cache.insert(key.to_vec(), value.to_vec());
            true
        }
    }

    fn read(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.alterations.borrow_mut().push(Access::init(key.to_vec(), AccessType::Read));

        if self.cache.contains_key(key) {
            self.cache.get(key).cloned()
        } else {
            self.state_reference.read(key)
        }
    }

    fn update(&mut self, key: &[u8], value: &Vec<u8>) -> Option<Vec<u8>> {
        self.alterations.borrow_mut().push(Access::init(key.to_vec(), AccessType::Write));

        self.cache.insert(key.to_vec(), value.to_vec())
    }

    fn delete(&mut self, key: &Vec<u8>) -> Option<Vec<u8>> {
        self.alterations.borrow_mut().push(Access::init(key.to_vec(), AccessType::Delete));

        self.cache.remove(key)
    }
}

/// Execute the given batch in a scalable manner, utilizing a thread pool, performing collision analysis
fn scalable_execution<A, S>(thread_pool: &mut Pool, application: &A, state: &mut S, batch: UpdateBatch<A::AppData::Request>) -> BatchReplies<Reply<A, S>>
    where A: Application<S>, S: CRUDState {
    let seq_no = batch.sequence_number();

    let mut execution_results = BTreeMap::new();

    let mut replies = BatchReplies::with_capacity(batch.len());

    let mut execution_units = Vec::with_capacity(batch.len());

    let mut updates = batch.into_inner();

    let mut collision_state = CollisionState {
        accessed: Default::default(),
        collisions: Default::default(),
    };

    let (tx, rx) = channel::new_bounded_sync(batch.len());

    thread_pool.scoped(|scope| {
        updates.iter().enumerate().for_each(|(pos, request)| {
            scope.execute(move || {
                let (exec_unit, reply) = speculative_execution::<A, S>(application, seq_no, pos, state, request.operation().clone());

                tx.clone().send((pos, exec_unit, UpdateReply::init(request.from(), request.session_id(), request.operation_id(), reply))).unwrap();
            });
        });

        while let Ok((pos, exec_unit, reply)) = rx.recv() {
            progress_collision_state(&mut collision_state, &exec_unit);

            execution_results.insert(pos, (exec_unit, reply));
        }
    });

    for collision in collision_state.collisions {

        // Discard of the results since we have they have collided
        execution_results.remove(&collision);

        let update = &updates[collision];

        let app_reply = application.update(state, update.operation().clone());

        replies.add(update.from(), update.session_id(), update.operation_id(), app_reply);
    }

    let mut to_apply = Vec::with_capacity(execution_results.len());

    for (pos, (exec_unit, reply)) in execution_results {
        to_apply.push(exec_unit);

        replies.push(reply);
    }

    apply_results_to_state(state, to_apply);

    replies
}

/// Unordered execution scales better than ordered execution and does not require collision verification
fn scalable_unordered_execution<A, S>(thread_pool: &mut Pool, application: &A, state: &S, batch: UnorderedBatch<A::AppData::Request>) -> BatchReplies<A::AppData::Reply>
    where A: Application<S> {
    let mut replies = BatchReplies::with_capacity(batch.len());
    let (tx, rx) = channel::new_bounded_sync(batch.len());

    thread_pool.scoped(|scope| {
        batch.iter().enumerate().for_each(|(pos, request)| {
            scope.execute(move || {
                let reply = speculatively_execute_unordered::<A, S>(application, state, request.operation().clone());

                tx.clone().send( UpdateReply::init(request.from(), request.session_id(), request.operation_id(), reply)).unwrap();
            });
        });

        while let Ok(reply) = rx.recv() {
            replies.push(reply);
        }
    });

    replies
}

/// Apply the given results of execution to the state
fn apply_results_to_state<S>(state: &mut S, execution_units: Vec<ExecutionUnit<S>>) where S: CRUDState {
    for unit in execution_units {
        unit.alterations.borrow().iter().for_each(|alteration| {
            match alteration.access_type {
                AccessType::Read => {
                    // Ignore read accesses as they do not affect the state
                }
                AccessType::Write => {

                    //TODO: If this repeats various write accesses to the same key,
                    // Reduce them all into a single access
                    unit.cache.get(&alteration.key).map(|value| {
                        state.update(&alteration.key, value);
                    });
                }
                AccessType::Delete => {
                    state.delete(&alteration.key)
                }
            }
        });
    }
}

/// Speculatively execute an unordered operation
fn speculatively_execute_unordered<A, S>(application: &A, state: &S, request: A::AppData::Request) -> A::AppData::Reply
    where A: Application<S> {
    application.unordered_execution(state, request)
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

pub struct Collisions {
    /// The indexes within the batch of the operations
    collided: BTreeSet<usize>,
}

struct CollisionState {
    accessed: HashMap<Vec<u8>, (BTreeSet<AccessType>, BTreeSet<usize>)>,
    collisions: BTreeSet<usize>,
}

/// Progress the collision state with the given execution unit
/// This allows the thread to not be waiting the finish of all tasks before actually starting to
/// calculate the collisions
fn progress_collision_state<S>(state: &mut CollisionState, unit: &ExecutionUnit<S>) -> bool where S: CRUDState {
    let mut collided = false;

    unit.alterations.borrow().iter().for_each(|access| {
        if let Some((accesses, operation_seq)) = state.accessed.get_mut(&access.key) {
            for access_type in accesses {
                if access_type.is_collision(&access.access_type) {
                    state.collisions.insert(unit.position_in_batch);

                    for seq in operation_seq {
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

            state.accessed.insert(access.key.clone(), (accesses, accessors));
        }
    });

    collided
}

/// Calculate collisions of data accesses within a batch, returns all
/// of the operations that must be executed sequentially
fn calculate_collisions<S>(execution_units: Vec<ExecutionUnit<S>>) -> Option<Collisions> {
    let mut accessed: HashMap<Vec<u8>, (Vec<AccessType>, Vec<usize>)> = Default::default();

    let mut collisions = BTreeSet::new();

    for unit in execution_units {
        unit.alterations.borrow().iter().for_each(|access| {
            if let Some((accesses, operation_seq)) = accessed.get_mut(&access.key) {
                for access_type in accesses {
                    if access_type.is_collision(&access.access_type) {
                        collisions.insert(unit.position_in_batch);

                        for seq in operation_seq {
                            collisions.insert(*seq);
                        }
                    }
                }

                operation_seq.push(unit.position_in_batch);
                accesses.push(access.access_type);
            } else {
                accessed.insert(access.key.clone(), (vec![access.access_type], vec![unit.position_in_batch]));
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

impl Access {
    fn init(key: Vec<u8>, access_type: AccessType) -> Self {
        Self {
            key,
            access_type,
        }
    }
}

impl AccessType {
    fn is_collision(&self, access_type: &AccessType) -> bool {
        match (self, access_type) {
            // Write - Anything is a collision
            (AccessType::Write, _) | (_, AccessType::Write) => true,
            // Delete - Anything is also a collision
            (AccessType::Delete, _) | (_, AccessType::Delete) => true,
            (_, _) => false
        }
    }
}