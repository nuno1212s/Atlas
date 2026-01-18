use atlas_core::ordering_protocol::decision::DecisionRequestBatch;

/// Trait that defines the necessary behaviour of a logged decisions handle for any given application.
pub trait TLoggedDecisionsHandle<RQ>: Send + Clone + 'static {

    /// Registers that a decision has been logged.
    fn register_decisions_logged(&self, decision: DecisionRequestBatch<RQ>) -> atlas_common::error::Result<()>;

}