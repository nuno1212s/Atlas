use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};

pub mod app;
pub mod deterministic_execution;
pub mod preemptive_execution;
pub mod serialize;
pub mod state;

/// Trait representing a handle to the client request execution.
pub trait TExecutionHandle<RQ>: Clone + Send {
    /// Instructs the execution to poll the state channel for updates.
    ///
    /// Used to notify the execution of incoming state updates.
    fn poll_state_channel(&self) -> Result<()>;

    /// Catches up the execution to the current state by executing the given requests.
    fn catch_up_to_quorum(&self, requests: MaybeVec<UpdateBatch<RQ>>) -> Result<()>;

    /// Queues a batch of requests `batch` for execution.
    fn queue_unordered(&self, requests: UnorderedUpdateBatch<RQ>) -> Result<()>;
}
