use crate::decision_log::serialize::DecisionLogMessage;
use crate::decision_log::{DecLog, DecLogMetadata, LoggingDecision};
use atlas_common::ordering::SeqNo;
use atlas_common::serialization_helper::SerMsg;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::PProof;
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_core::ordering_protocol::BatchedDecision;
use atlas_core::persistent_log::{OperationMode, OrderingProtocolLog};

/// The trait that defines the the persistent decision log, so that the decision log can be persistent
pub trait PersistentDecisionLog<RQ, OPM, POP, LS>: OrderingProtocolLog<RQ, OPM> + Send
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POP>,
{
    /// A checkpoint has been done on the state, so we can clear the current decision log
    fn checkpoint_received(
        &self,
        mode: OperationMode,
        seq: SeqNo,
    ) -> atlas_common::error::Result<()>;

    /// Write a given proof to the persistent log
    fn write_proof(
        &self,
        write_mode: OperationMode,
        proof: PProof<RQ, OPM, POP>,
    ) -> atlas_common::error::Result<()>;

    /// Write the metadata of a decision into the persistent log
    fn write_decision_log_metadata(
        &self,
        mode: OperationMode,
        log_metadata: DecLogMetadata<RQ, OPM, POP, LS>,
    ) -> atlas_common::error::Result<()>;

    /// Write the decision log into the persistent log
    fn write_decision_log(
        &self,
        mode: OperationMode,
        log: DecLog<RQ, OPM, POP, LS>,
    ) -> atlas_common::error::Result<()>;

    /// Read a proof from the log with the given sequence number
    fn read_proof(
        &self,
        mode: OperationMode,
        seq: SeqNo,
    ) -> atlas_common::error::Result<Option<PProof<RQ, OPM, POP>>>;

    /// Read the decision log from the persistent storage
    fn read_decision_log(
        &self,
        mode: OperationMode,
    ) -> atlas_common::error::Result<Option<DecLog<RQ, OPM, POP, LS>>>;

    /// Reset the decision log on disk
    fn reset_log(&self, mode: OperationMode) -> atlas_common::error::Result<()>;

    /// Wait for the persistence of a given proof, if necessary
    /// The return of this function is dependent on the current mode of the persistent log.
    /// Namely, if we have to perform some sort of operations before the decision can be safely passed
    /// to the executor, then we want to return [None] on this function. If there is no need
    /// of further persistence, then the decision should be re returned with
    /// [Some(ProtocolConsensusDecision<D::Request>)]
    fn wait_for_full_persistence(
        &self,
        batch: BatchedDecision<RQ>,
        logging_decision: LoggingDecision,
    ) -> atlas_common::error::Result<Option<BatchedDecision<RQ>>>;
}
