use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::SeqNo;
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::StoredMessage;

use crate::decision_log::{DecisionLog, LoggedDecision};
use crate::log_transfer::networking::serialize::LogTransferMessage;
use crate::log_transfer::networking::LogTransferSendNode;
use crate::persistent_log::PersistentDecisionLog;
use atlas_core::ordering_protocol::loggable::LoggableOrderProtocol;
use atlas_core::ordering_protocol::networking::serialize::NetworkView;
use atlas_core::timeouts::timeout::{TimeoutModHandle, TimeoutableMod};

pub mod networking;

pub type LogTM<RQ, OP, M: LogTransferMessage<RQ, OP>> = <M as LogTransferMessage<RQ, OP>>::LogTransferMessage;

/// The result of processing a message in the log transfer protocol
#[allow(clippy::large_enum_variant)]
pub enum LTResult<RQ> {
    RunLTP,
    NotNeeded,
    Running,
    Ignored,
    /// The log transfer protocol has reached a point where we can already say which is
    /// the current seq number of the quorum
    InstallSeq(SeqNo),
    /// The log transfer protocol has finished and the ordering protocol should now
    /// be proceeded. The requests contained are requests that must be executed by the application
    /// in order to reach the state that corresponds to the decision log
    /// FirstSeq and LastSeq of the installed log downloaded from other replicas and the requests that should be executed
    LTPFinished(SeqNo, SeqNo, MaybeVec<LoggedDecision<RQ>>),
}

/// Log Transfer protocol timeout result
pub enum LTTimeoutResult {
    RunLTP,
    NotNeeded,
}

/// Log Transfer polling result
pub enum LTPollResult<LT, RQ> {
    ReceiveMsg,
    RePoll,
    Exec(StoredMessage<LT>),
    LTResult(LTResult<RQ>),
}

/// Log transfer protocol.
/// This protocol is meant to work in tandem with the [DecisionLog] abstraction, meaning
/// it has to actually "hook" into it and directly use functions from the DecisionLog.
/// Examples are, for example using the [DecisionLog::snapshot_log] when wanting a snapshot
/// of the current log, [DecisionLog::install_log] when we want to install a log that
/// we have received.
/// This level of coupling exists because we need to be able to reference the decision log
/// in order to obtain the current log (cloning it and passing it to this function every time
/// would be extremely expensive) and since this protocol only makes sense when paired with
/// the [DecisionLog], we decided that it makes sense for them to be more tightly coupled.
///TODO: Work on Getting partial log installations integrated with this log transfer
/// trait via [PartiallyWriteableDecLog]
pub trait LogTransferProtocol<RQ, OP, DL>: TimeoutableMod<LTTimeoutResult> + Send
where
    RQ: SerMsg,
    OP: LoggableOrderProtocol<RQ>,
    DL: DecisionLog<RQ, OP>,
{
    /// The type which implements StateTransferMessage, to be implemented by the developer
    type Serialization: LogTransferMessage<RQ, OP::Serialization> + 'static;

    /// The configuration type the protocol wants to accept
    type Config: Send + 'static;

    /// Request the latest logs from the rest of the replicas
    fn request_latest_log<V>(&mut self, decision_log: &mut DL, view: V) -> Result<()>
    where
        V: NetworkView;

    /// Polling method for the log transfer protocol
    fn poll(
        &mut self,
    ) -> Result<LTPollResult<LogTM<RQ, OP::Serialization, Self::Serialization>, RQ>>;

    /// Handle a state transfer protocol message that was received while executing the ordering protocol
    fn handle_off_ctx_message<V>(
        &mut self,
        decision_log: &mut DL,
        view: V,
        message: StoredMessage<LogTM<RQ, OP::Serialization, Self::Serialization>>,
    ) -> Result<()>
    where
        V: NetworkView;

    /// Process a log transfer protocol message, received from other replicas
    fn process_message<V>(
        &mut self,
        decision_log: &mut DL,
        view: V,
        message: StoredMessage<LogTM<RQ, OP::Serialization, Self::Serialization>>,
    ) -> Result<LTResult<RQ>>
    where
        V: NetworkView;
}

/// The complete trait for a log transfer protocol initializer initialization.
///
pub trait LogTransferProtocolInitializer<RQ, OP, DL, PL, EX, NT>:
    LogTransferProtocol<RQ, OP, DL>
where
    RQ: SerMsg,
    OP: LoggableOrderProtocol<RQ>,
    DL: DecisionLog<RQ, OP>,
{
    fn initialize(
        config: Self::Config,
        timeout: TimeoutModHandle,
        node: Arc<NT>,
        log: PL,
    ) -> Result<Self>
    where
        Self: Sized,
        PL: PersistentDecisionLog<
            RQ,
            OP::Serialization,
            OP::PersistableTypes,
            DL::LogSerialization,
        >,
        NT: LogTransferSendNode<RQ, OP::Serialization, Self::Serialization>;
}

impl<RQ: SerMsg> Debug for LTResult<RQ> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LTResult::RunLTP => {
                write!(f, "RunLTP")
            }
            LTResult::NotNeeded => {
                write!(f, "NotNeeded")
            }
            LTResult::Running => {
                write!(f, "Running")
            }
            LTResult::LTPFinished(first, last, _) => {
                write!(f, "LTPFinished({first:?}, {last:?})")
            }
            LTResult::InstallSeq(seq) => {
                write!(f, "LTPInstallSeq({seq:?})")
            }
            LTResult::Ignored => {
                write!(f, "Ignored Message")
            }
        }
    }
}
