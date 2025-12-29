pub mod preemptive_execution;
pub mod deterministic_execution;
pub mod requests;

use crate::ordering_protocol::decision::BatchedDecision;
use atlas_common::maybe_vec::MaybeVec;
use atlas_communication::message::StoredMessage;

/// Trait that defines the necessary behaviour of a execution handle for any given application.
///
/// Execution handles mean the channel through which the protocol should send requests to the execution (whichever execution
/// that may be).
pub trait TDecisionExecutorHandle<RQ>: Send + Clone + 'static {
    /// Queues a vec of decisions for execution.
    fn catch_up_to_quorum(
        &self,
        requests: MaybeVec<BatchedDecision<RQ>>,
    ) -> atlas_common::error::Result<()>;

    /// Queues a batch of unordered requests for execution
    fn queue_update_unordered(
        &self,
        requests: Vec<StoredMessage<RQ>>,
    ) -> atlas_common::error::Result<()>;
}

