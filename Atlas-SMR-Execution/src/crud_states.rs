use getset::{CopyGetters, Getters};
use atlas_smr_application::app::{Application, Reply, Request};

/// A trait defining the CRUD operations required to be implemented for a given state in order
/// for it to be utilized as a scalable state.
pub trait CRUDState: Send {
    /// Read an entry from the state
    fn read(&self, column: &str, key: &[u8]) -> Option<Vec<u8>>;

    /// Create a new entry in the state
    fn create(&mut self, column: &str, key: &[u8], value: &[u8]) -> bool;

    /// Update an entry in the state
    /// Returns the previous value that was stored in the state
    fn update(&mut self, column: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>>;

    /// Delete an entry in the state
    fn delete(&mut self, column: &str, key: &[u8]) -> Option<Vec<u8>>;
}

/// A trait defining the methods required for an application to be scalable
pub trait CRUDApplication<S>: Application<S> + Sync
where
    S: CRUDState,
{
    /// This execution method takes a dynamic reference to the state. This is because
    /// We will pass it an ExecutionUnit which is not S, so it must handle any state that
    /// implements CRUDState.
    /// This operation must yield the same result as the ordered execution of the same request
    /// for this to correctly function.
    fn speculatively_execute(
        &self,
        state: &mut impl CRUDState,
        request: Request<Self, S>,
    ) -> Reply<Self, S>;
}

#[derive(Getters, CopyGetters)]
pub struct Access {
    #[get = "pub"]
    column: String,
    #[get = "pub"]
    key: Vec<u8>,
    #[get_copy = "pub"]
    access_type: AccessType,
}

impl Access {
    pub fn new(column: &str, key: Vec<u8>, access_type: AccessType) -> Self {
        Self {
            column: column.to_string(),
            key,
            access_type,
        }
    }
}

/// Types of accesses to data stored in the state
#[derive(Copy, Clone, Debug, PartialOrd, PartialEq, Eq, Ord)]
pub enum AccessType {
    Read,
    Write,
    Delete,
}

impl AccessType {
    pub fn is_collision(&self, access_type: &AccessType) -> bool {
        match (self, access_type) {
            // Write - Anything is a collision
            (AccessType::Write, _) | (_, AccessType::Write) => true,
            // Delete - Anything is also a collision
            (AccessType::Delete, _) | (_, AccessType::Delete) => true,
            (_, _) => false,
        }
    }
}
