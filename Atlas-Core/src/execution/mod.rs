pub mod requests;

use crate::ordering_protocol::decision::DecisionRequestBatch;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::StoredMessage;

/// Trait that defines the necessary behaviour of a execution handle for any given application.
///
/// Execution handles mean the channel through which the protocol should send requests to the execution (whichever execution
/// that may be).
pub trait TExecutorDecisionHandle<RQ>: Send + Clone + 'static {
    /// Queues a vec of decisions for execution.
    fn catch_up_to_quorum(
        &self,
        requests: MaybeVec<DecisionRequestBatch<RQ>>,
    ) -> atlas_common::error::Result<()>;

    /// Queues a batch of unordered requests for execution
    fn queue_update_unordered(
        &self,
        requests: Vec<StoredMessage<RQ>>,
    ) -> atlas_common::error::Result<()>;
}

/// Core decision execution abstraction.
///
/// Takes decisions that were output by the ordering protocol and executes them.
pub trait TDeterministicExecutorDecisionHandle<RQ> : TExecutorDecisionHandle<RQ> {

    /// Queues a batch of requests `batch` for execution.
    /// 
    /// The requests in this batch have been finalized by the consensus protocol
    fn queue_update(&self, batch: DecisionRequestBatch<RQ>) -> atlas_common::error::Result<()>;

}

/// Trait describing the behaviour of a preemptive decision executor handle.
///
/// Preemptive decision executors are able to execute preemptive updates, which are updates that can be
/// executed before the consensus has effectively finalized them, meaning we are capable of executing them
/// in parallel with the decision making process reducing latency at the cost of potentially having to roll back some
/// of these updates if they end up not being finalized.
/// 
/// When a preemptive update is finalized, the executor is notified via `queue_preemptive_update_finalized`.
/// When we receive a queue update with a seq number that has already been preemptively executed, the executor
/// should discard all work already done for that update (and for later updates that were preemptively executed) and
/// re-execute them in order to ensure determinism as the preemptive execution may have diverged from the finalized execution.
pub trait TPreemptiveExecutorDecisionHandle<RQ> : TDeterministicExecutorDecisionHandle<RQ> {

    /// Queues a preemptive update batch for execution.
    fn queue_preemptive_update(&self, batch: DecisionRequestBatch<RQ>) -> atlas_common::error::Result<()>;

    /// Finalizes the preemptive update identified by `seq`.
    fn queue_preemptive_update_finalized(&self, seq: SeqNo) -> atlas_common::error::Result<()>;

}
