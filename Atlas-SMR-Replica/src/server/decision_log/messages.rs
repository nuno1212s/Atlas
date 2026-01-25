use crate::server::decision_log::DecisionShort;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::StoredMessage;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::{PProof, TLoggableOrderProtocol};
use atlas_core::ordering_protocol::networking::serialize::{NetworkView, OrderingProtocolMessage};
use atlas_core::timeouts::timeout::ModTimeout;
use atlas_logging_core::decision_log::TDecisionLog;
use atlas_logging_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_logging_core::log_transfer::{LogTM, LogTransferProtocol};
use atlas_smr_core::SMRRawReq;
use std::fmt::{Debug, Formatter};

pub type DLWorkMessageShort<
    V: NetworkView,
    R: SerMsg,
    OP: TLoggableOrderProtocol<SMRRawReq<R>>,
    LT: LogTransferProtocol<SMRRawReq<R>, OP, DL>,
    DL: TDecisionLog<SMRRawReq<R>, OP>,
> = DLWorkMessage<V, SMRRawReq<R>, OP::Serialization, OP::PersistableTypes, LT::Serialization>;

#[allow(dead_code, clippy::large_enum_variant)]
pub enum DecisionLogWorkMessage<RQ, OPM, POT>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
{
    ClearSequenceNumber(SeqNo),
    ClearUnfinishedDecisions,
    DecisionInformation(MaybeVec<DecisionShort<RQ, OPM>>),
    Proof(PProof<RQ, OPM, POT>),
    CheckpointDone(SeqNo),
}

impl<RQ, OPM, POT> Debug for DecisionLogWorkMessage<RQ, OPM, POT>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionLogWorkMessage::ClearSequenceNumber(seq) => {
                write!(f, "Clear sequence number: {seq:?}")
            }
            DecisionLogWorkMessage::ClearUnfinishedDecisions => {
                write!(f, "Clear unfinished decisions")
            }
            DecisionLogWorkMessage::DecisionInformation(dec_info) => {
                write!(f, "Decision information: {dec_info:?}")
            }
            DecisionLogWorkMessage::Proof(proof) => {
                write!(f, "Proof: {:?}", proof.sequence_number())
            }
            DecisionLogWorkMessage::CheckpointDone(seq) => {
                write!(f, "Checkpoint done: {seq:?}")
            }
        }
    }
}

pub struct DLWorkMessage<V, RQ, OPM, POT, LTM>
where
    V: NetworkView,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    pub(super) view: V,
    pub(super) message: DLWorkMessageType<RQ, OPM, POT, LTM>,
}

impl<V, RQ, OPM, POT, LTM> DLWorkMessage<V, RQ, OPM, POT, LTM>
where
    V: NetworkView,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    pub fn initialize_message(view: V, work_msg: DLWorkMessageType<RQ, OPM, POT, LTM>) -> Self {
        Self {
            view,
            message: work_msg,
        }
    }

    pub fn init_log_transfer_message(
        view: V,
        work_msg: LogTransferWorkMessage<RQ, OPM, LTM>,
    ) -> Self {
        Self::initialize_message(view, DLWorkMessageType::LogTransfer(work_msg))
    }

    pub fn init_dec_log_message(view: V, work_msg: DecisionLogWorkMessage<RQ, OPM, POT>) -> Self {
        Self::initialize_message(view, DLWorkMessageType::DecisionLog(work_msg))
    }
}

/// Messages that are destined to the replica so it can piece
/// together the current state of the decision log
pub enum ReplicaWorkResponses {
    InstallSeqNo(SeqNo),
    LogTransferFinalized(SeqNo, SeqNo),
    LogTransferNotNeeded(SeqNo, SeqNo),
}

#[derive()]
pub enum LogTransferWorkMessage<RQ, OPM, LTM>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    RequestLogTransfer,
    LogTransferMessage(StoredMessage<LogTM<RQ, OPM, LTM>>),
    ReceivedTimeout(Vec<ModTimeout>),
    TransferDone(SeqNo, SeqNo),
}

impl<RQ, OPM, LTM> Debug for LogTransferWorkMessage<RQ, OPM, LTM>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LogTransferWorkMessage::RequestLogTransfer => {
                write!(f, "Request log transfer")
            }
            LogTransferWorkMessage::LogTransferMessage(message) => {
                write!(f, "Log transfer message: {:?}", message.header())
            }
            LogTransferWorkMessage::ReceivedTimeout(timeout) => {
                write!(f, "Received timeout: {timeout:?}")
            }
            LogTransferWorkMessage::TransferDone(start, end) => {
                write!(f, "Transfer done: {start:?} - {end:?}")
            }
        }
    }
}

pub enum DLWorkMessageType<RQ, OPM, POT, LTM>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    DecisionLog(DecisionLogWorkMessage<RQ, OPM, POT>),
    LogTransfer(LogTransferWorkMessage<RQ, OPM, LTM>),
}
