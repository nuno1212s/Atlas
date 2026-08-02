use crate::TExecutionHandle;
use atlas_core::execution::requests::UpdateBatch;

/// Trait representing a handle to the client request execution
/// with deterministic execution capabilities.
pub trait TDeterministicExecutionHandle<RQ>: TExecutionHandle<RQ> {
    /// Queues a batch of requests `batch` for execution.
    fn queue_update(&self, batch: UpdateBatch<RQ>) -> atlas_common::error::Result<()>;

    /// Same as `queue_update()`, additionally reporting the serialized
    /// application state.
    ///
    /// This is useful during local checkpoints.
    fn queue_update_and_get_appstate(
        &self,
        batch: UpdateBatch<RQ>,
    ) -> atlas_common::error::Result<()>;
}
