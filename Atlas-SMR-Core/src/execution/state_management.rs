use atlas_common::ordering::SeqNo;
use atlas_core::ordering_protocol::decision::BatchedDecision;

/// Trait that defines the necessary behaviour of an executor state handle.
pub trait TExecutorStateHandle<RQ> {
    
    /// Instruct the executor to poll the state channel for new state updates.
    fn poll_state_channel(&self) -> atlas_common::error::Result<()>;

}

/// Trait that defines the necessary behaviour of a deterministic executor state handle.
pub trait TDeterministicExecutorStateHandle<RQ>: TExecutorStateHandle<RQ> {

    /// Queue an update identified by `seq` and retrieve the application state after the update.
    fn queue_update_and_get_appstate(&self, batch: BatchedDecision<RQ>) -> atlas_common::error::Result<()>;
    
}

pub trait TPreemptiveExecutorStateHandle<RQ>: TDeterministicExecutorStateHandle<RQ> {

    /// Queue an update identified by `seq` and retrieve the application state after the update.
    fn queue_update_finalized_and_get_appstate(&self, seq: SeqNo) -> atlas_common::error::Result<()>;

}