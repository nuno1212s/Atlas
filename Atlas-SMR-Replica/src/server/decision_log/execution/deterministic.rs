use crate::server::decision_log::execution::TDecisionLogExecution;
use atlas_common::ordering::Orderable;
use atlas_core::execution::{TDeterministicExecutorDecisionHandle, TPreemptiveDecisionExecutorHandle};
use atlas_core::ordering_protocol::decision::BatchedDecision;
use atlas_smr_core::execution::state_management::{TDeterministicExecutorStateHandle, TPreemptiveExecutorStateHandle};

/// Deterministic decision executor for the decision log.
pub struct DeterministicExecutor<E> {
    executor_handle: E,
}

impl<E> Clone for DeterministicExecutor<E>
where
    E: Clone,
{
    fn clone(&self) -> Self {
        Self {
            executor_handle: self.executor_handle.clone(),
        }
    }
}

impl<E, RQ> TDecisionLogExecution<RQ> for DeterministicExecutor<E>
where
    E: TDeterministicExecutorDecisionHandle<RQ> + TDeterministicExecutorStateHandle<RQ>,
{
    fn send_for_execution(&self, decision: BatchedDecision<RQ>) -> atlas_common::error::Result<()> {
        self.executor_handle.queue_update(decision)
    }

    fn send_for_execution_and_get_appstate(&self, decision: BatchedDecision<RQ>) -> atlas_common::error::Result<()> {
        self.executor_handle.queue_update_and_get_appstate(decision)
    }
}

pub struct PreemptiveExecutor<E> {
    executor_handle: E,
}

impl<E> Clone for PreemptiveExecutor<E>
where E: Clone
{
    fn clone(&self) -> Self {
        Self {
            executor_handle: self.executor_handle.clone(),
        }
    }
}

impl<E, RQ> TDecisionLogExecution<RQ> for PreemptiveExecutor<E>
where
    E: TPreemptiveDecisionExecutorHandle<RQ> + TPreemptiveExecutorStateHandle<RQ> + Send + 'static,
{
    fn send_for_execution(&self, decision: BatchedDecision<RQ>) -> atlas_common::error::Result<()> {
        self.executor_handle.queue_preemptive_update_finalized(decision.sequence_number())
    }

    fn send_for_execution_and_get_appstate(&self, decision: BatchedDecision<RQ>) -> atlas_common::error::Result<()> {
        self.executor_handle.queue_update_finalized_and_get_appstate(decision.sequence_number())
    }
}