use atlas_common::error::Result;
use atlas_core::ordering_protocol::decision::BatchedDecision;

mod deterministic;
mod preemptive;

/// Decision execution abstraction for the decision log.
///
/// Made to support both deterministic and preemptive execution models.
pub trait TDecisionLogExecution<RQ> : Send + Clone {

    fn send_for_execution(&self, decision: BatchedDecision<RQ>) -> Result<()>;

    fn send_for_execution_and_get_appstate(&self, decision: BatchedDecision<RQ>) -> Result<()>;

}