use atlas_common::ordering::SeqNo;
use atlas_common::error::*;
use atlas_core::execution::requests::UpdateBatch;
use crate::TExecutionHandle;

/// A trait for execution handles that support preemptive execution.
///
/// This trait extends the `TExecutionHandle` trait with methods specific to
/// preemptive execution, allowing for queuing preemptive updates and finalizing
/// updates.
pub trait TPreemptiveExecutionHandle<RQ> : TExecutionHandle<RQ> {

    /// Queues a preemptive update batch for execution.
    fn queue_preemptive_update(&self, batch: UpdateBatch<RQ>) -> Result<()>;

    /// Finalizes the preemptive update identified by `seq`.
    fn queue_update_finalized(&self, seq: SeqNo) -> Result<()>;

    /// Finalizes the preemptive update identified by `seq` and retrieves the
    /// application state after the update.
    fn queue_update_finalized_and_get_appstate(&self, seq: SeqNo) -> Result<()>;

}