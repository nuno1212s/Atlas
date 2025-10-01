pub mod serialize;

use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::decision_log::serialize::DecisionLogMessage;
use crate::persistent_log::PersistentDecisionLog;
use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::StoredMessage;
use atlas_core::executor::DecisionExecutorHandle;
use atlas_core::messages::ClientRqInfo;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::{LoggableOrderProtocol, PProof};
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_core::ordering_protocol::{
    BatchedDecision, Decision, DecisionAD, DecisionMetadata, ProtocolMessage,
    ShareableConsensusMessage,
};

pub type DecLog<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POP>,
> = LS::DecLog;
pub type DecLogMetadata<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POP>,
> = LS::DecLogMetadata;

pub type DecLogPart<
    RQ: SerMsg,
    OP: LoggableOrderProtocol<RQ>,
    LS: DecisionLogMessage<RQ, OP::Serialization, OP::PersistableTypes>,
> = LS::DecLogPart;

/// Type aliases for complex types
pub type DecisionType<RQ: SerMsg, OP: LoggableOrderProtocol<RQ>> = Decision<
    DecisionMetadata<RQ, OP::Serialization>,
    DecisionAD<RQ, OP::Serialization>,
    ProtocolMessage<RQ, OP::Serialization>,
    RQ,
>;

pub type ProofType<RQ: SerMsg, OP: LoggableOrderProtocol<RQ>> =
    PProof<RQ, OP::Serialization, OP::PersistableTypes>;
pub type DecisionLogType<
    RQ: SerMsg,
    OP: LoggableOrderProtocol<RQ>,
    LS: DecisionLogMessage<RQ, OP::Serialization, OP::PersistableTypes>,
> = DecLog<RQ, OP::Serialization, OP::PersistableTypes, LS>;

pub type LogMetadataType<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POP>,
> = DecLogMetadata<RQ, OPM, POP, LS>;
pub type ProofVecType<RQ, OPM, POP: PersistentOrderProtocolTypes<RQ, OPM>> =
    Vec<PProof<RQ, OPM, POP>>;
pub type RefProofVecType<'a, RQ, OPM, POP: PersistentOrderProtocolTypes<RQ, OPM>> =
    Vec<&'a PProof<RQ, OPM, POP>>;

/// The record of the decision that has been made.
#[derive(Clone)]
pub struct LoggedDecision<O> {
    // The sequence number
    seq: SeqNo,
    // The client requests that were contained in the decision
    contained_client_requests: Vec<ClientRqInfo>,
    decision_value: LoggedDecisionValue<O>,
}

/// Contains the requests that were in the logged decision,
/// in the case we want the replica to handle the execution
/// If we return [LoggedDecisionValue<O>::ExecutionNotNeeded],
/// we assume that the execution handling of the requests will be done
/// by the decision log
#[derive(Clone)]
pub enum LoggedDecisionValue<O> {
    Execute(BatchedDecision<O>),
    ExecutionNotNeeded,
}

/// The information about a decision that is part of the decision log.
/// Namely, the sequence number and the messages that must be stored
/// for that sequence number proof to be completely stored.
/// This is what is used to handle the Strict persistency mode,
/// among other necessary
pub enum LoggingDecision {
    Proof(SeqNo),
    PartialDecision(SeqNo, Vec<(NodeId, Digest)>),
}

pub trait RangeOrderable: Orderable {
    fn first_sequence(&self) -> SeqNo;
}

/// The abstraction for the SMR decision log
/// All SMR systems require this decision log since they function
/// on a Checkpoint based approach so it naturally requires
/// the knowledge of all decisions since the last checkpoint in order
/// to both recover replicas or integrate new replicas into the system
///
/// IMPORTANT: Refer to [LoggedDecision<O>] in order to better understand
/// how to handle logged decision execution
///
/// Also important, the [Orderable] trait implemented here should return the sequence
/// number of the last DECIDED decision, not of the ongoing decisions
pub trait DecisionLog<RQ, OP>:
    RangeOrderable
    + DecisionLogPersistenceHelper<
        RQ,
        OP::Serialization,
        OP::PersistableTypes,
        Self::LogSerialization,
    >
where
    RQ: SerMsg,
    OP: LoggableOrderProtocol<RQ>,
{
    /// The serialization type containing the serializable parts for the decision log
    type LogSerialization: DecisionLogMessage<RQ, OP::Serialization, OP::PersistableTypes> + 'static;

    type Config: Send + 'static;

    /// Clear the sequence number in the decision log
    fn clear_sequence_number(&mut self, seq: SeqNo) -> Result<()>;

    /// Clear all decisions forward of the provided one (inclusive)
    fn clear_decisions_forward(&mut self, seq: SeqNo) -> Result<()>;

    /// The given sequence number was advanced in state with the given
    fn decision_information_received(
        &mut self,
        decision_info: DecisionType<RQ, OP>,
    ) -> Result<MaybeVec<LoggedDecision<RQ>>>;

    /// Install an entire proof into the decision log.
    fn install_proof(&mut self, proof: ProofType<RQ, OP>) -> Result<MaybeVec<LoggedDecision<RQ>>>;

    /// Install a log received from other replicas in the system
    fn install_log(
        &mut self,
        dec_log: DecisionLogType<RQ, OP, Self::LogSerialization>,
    ) -> Result<MaybeVec<LoggedDecision<RQ>>>;

    /// Take a snapshot of our current decision log.
    fn snapshot_log(&mut self) -> Result<DecisionLogType<RQ, OP, Self::LogSerialization>>;

    /// Get the reference to the current log
    fn current_log(&self) -> Result<&DecisionLogType<RQ, OP, Self::LogSerialization>>;

    /// A checkpoint has been done of the state
    fn state_checkpoint(&mut self, seq: SeqNo) -> Result<()>;

    /// Verify the sequence number sent by another replica
    fn verify_sequence_number(&self, seq_no: SeqNo, proof: &ProofType<RQ, OP>) -> Result<bool>;

    /// Get the current sequence number of the protocol with proof
    fn sequence_number_with_proof(&self) -> Result<Option<(SeqNo, ProofType<RQ, OP>)>>;

    /// Get the proof of decision for a given sequence number
    fn get_proof(&self, seq: SeqNo) -> Result<Option<ProofType<RQ, OP>>>;
}

pub trait PartiallyWriteableDecLog<RQ, OP>: DecisionLog<RQ, OP>
where
    RQ: SerMsg,
    OP: LoggableOrderProtocol<RQ>,
{
    fn start_installing_log(&mut self) -> Result<()>;

    fn install_log_part(
        &mut self,
        log_part: DecLogPart<RQ, OP, Self::LogSerialization>,
    ) -> Result<()>;

    fn complete_log_install(&mut self) -> Result<()>;
}

pub trait DecisionLogInitializer<RQ, OP, PL, EX>: DecisionLog<RQ, OP>
where
    RQ: SerMsg,
    OP: LoggableOrderProtocol<RQ>,
{
    /// Initialize the decision log of the
    fn initialize_decision_log(
        config: Self::Config,
        persistent_log: PL,
        executor_handle: EX,
    ) -> Result<Self>
    where
        PL: PersistentDecisionLog<
            RQ,
            OP::Serialization,
            OP::PersistableTypes,
            Self::LogSerialization,
        >,
        EX: DecisionExecutorHandle<RQ>,
        Self: Sized;
}

/// Persistence helper for the decision log
pub trait DecisionLogPersistenceHelper<RQ, OPM, POP, LS>: Send
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POP>,
{
    /// Initialize the decision log
    fn init_decision_log(
        metadata: LogMetadataType<RQ, OPM, POP, LS>,
        proofs: ProofVecType<RQ, OPM, POP>,
    ) -> Result<DecLog<RQ, OPM, POP, LS>>;

    /// Take a decision log and decompose it into parts
    fn decompose_decision_log(
        dec_log: DecLog<RQ, OPM, POP, LS>,
    ) -> (
        LogMetadataType<RQ, OPM, POP, LS>,
        ProofVecType<RQ, OPM, POP>,
    );

    /// Decompose a decision log into its parts, but only by references
    fn decompose_decision_log_ref(
        dec_log: &DecLog<RQ, OPM, POP, LS>,
    ) -> (
        &LogMetadataType<RQ, OPM, POP, LS>,
        RefProofVecType<RQ, OPM, POP>,
    );
}

/// Wrap a loggable message
pub fn wrap_loggable_message<RQ, OP, POP>(
    message: StoredMessage<ProtocolMessage<RQ, OP>>,
) -> ShareableConsensusMessage<RQ, OP>
where
    OP: OrderingProtocolMessage<RQ>,
{
    Arc::new(message)
}

impl<O> LoggedDecision<O> {
    pub fn from_decision(seq: SeqNo, client_rqs: Vec<ClientRqInfo>) -> Self {
        Self {
            seq,
            contained_client_requests: client_rqs,
            decision_value: LoggedDecisionValue::ExecutionNotNeeded,
        }
    }

    pub fn from_decision_with_execution(
        seq: SeqNo,
        client_rqs: Vec<ClientRqInfo>,
        update: BatchedDecision<O>,
    ) -> Self {
        Self {
            seq,
            contained_client_requests: client_rqs,
            decision_value: LoggedDecisionValue::Execute(update),
        }
    }

    pub fn into_inner(self) -> (SeqNo, Vec<ClientRqInfo>, LoggedDecisionValue<O>) {
        (
            self.seq,
            self.contained_client_requests,
            self.decision_value,
        )
    }
}

impl LoggingDecision {
    pub fn init_empty(seq: SeqNo) -> Self {
        Self::PartialDecision(seq, Vec::new())
    }

    pub fn init_from_proof(seq: SeqNo) -> Self {
        Self::Proof(seq)
    }

    pub fn insert_message<RQ, OP>(&mut self, message: &ShareableConsensusMessage<RQ, OP>)
    where
        OP: OrderingProtocolMessage<RQ>,
    {
        match self {
            LoggingDecision::PartialDecision(_, messages) => {
                messages.push((message.header().from(), *message.header().digest()))
            }
            LoggingDecision::Proof(_) => unreachable!(),
        }
    }
}

impl<O> Orderable for LoggedDecision<O> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl<O> Debug for LoggedDecision<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LoggedDecision {:?}, {} Client Rqs, {:?}",
            self.seq,
            self.contained_client_requests.len(),
            self.decision_value
        )
    }
}

impl<O> Debug for LoggedDecisionValue<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LoggedDecisionValue::Execute(_) => {
                write!(f, "Execute decs")
            }
            LoggedDecisionValue::ExecutionNotNeeded => {
                write!(f, "Exec not needed")
            }
        }
    }
}
