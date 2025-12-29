use crate::execution::TDecisionExecutorHandle;
use crate::ordering_protocol::decision::BatchedDecision;

/// Core decision execution abstraction.
/// 
/// Takes decisions that were output by the 
pub trait TDeterministicDecisionExecutorHandle<RQ> : TDecisionExecutorHandle<RQ> {

    /// Queues a batch of requests `batch` for execution.
    fn queue_update(&self, batch: BatchedDecision<RQ>) -> atlas_common::error::Result<()>;
    
}