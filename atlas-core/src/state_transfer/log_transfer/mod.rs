use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use atlas_common::error::*;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::StoredMessage;
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_execution::serialize::ApplicationData;
use crate::messages::{LogTransfer, StateTransfer};
use crate::ordering_protocol::{OrderingProtocol, OrderingProtocolArgs, SerProof, View};
use crate::ordering_protocol::stateful_order_protocol::StatefulOrderProtocol;
use crate::persistent_log::StatefulOrderingProtocolLog;
use crate::serialize::{LogTransferMessage, ServiceMsg, StatefulOrderProtocolMessage, StateTransferMessage};
use crate::timeouts::{RqTimeout, Timeouts};

pub type LogTM<M: LogTransferMessage> = <M as LogTransferMessage>::LogTransferMessage;

/// The result of processing a message in the log transfer protocol
pub enum LTResult<D: ApplicationData> {
    RunLTP,
    NotNeeded,
    Running,
    /// The log transfer protocol has finished and the ordering protocol should now
    /// be proceeded. The requests contained are requests that must be executed by the application
    /// in order to reach the state that corresponds to the decision log
    /// FirstSeq and LastSeq of the installed log downloaded from other replicas and the requests that should be executed
    LTPFinished(SeqNo, SeqNo, Vec<D::Request>),
}

pub enum LTTimeoutResult {
    RunLTP,
    NotNeeded,
}

/// The trait which defines the necessary methods for a log transfer protocol
pub trait LogTransferProtocol<D, OP, NT, PL> where D: ApplicationData + 'static,
                                                   OP: StatefulOrderProtocol<D, NT, PL> + 'static {
    /// The type which implements StateTransferMessage, to be implemented by the developer
    type Serialization: LogTransferMessage + 'static;

    /// The configuration type the protocol wants to accept
    type Config;

    /// Initialize the state transferring protocol with the given configuration, timeouts and communication layer
    fn initialize(config: Self::Config, timeouts: Timeouts, node: Arc<NT>, log: PL) -> Result<Self>
        where Self: Sized;

    /// Request the latest state from the rest of replicas
    fn request_latest_log<ST>(&mut self,
                              order_protocol: &mut OP) -> Result<()>
        where NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Self::Serialization>>,
              ST: StateTransferMessage + 'static,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization>;

    /// Handle a state transfer protocol message that was received while executing the ordering protocol
    fn handle_off_ctx_message<ST>(&mut self,
                                  order_protocol: &mut OP,
                                  message: StoredMessage<LogTransfer<LogTM<Self::Serialization>>>)
                                  -> Result<()>
        where NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Self::Serialization>>,
              ST: StateTransferMessage + 'static,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization>;

    /// Process a state transfer protocol message, received from other replicas
    /// We also provide a mutable reference to the stateful ordering protocol, so the
    /// state can be installed (if that's the case)
    fn process_message<ST>(&mut self,
                           order_protocol: &mut OP,
                           message: StoredMessage<LogTransfer<LogTM<Self::Serialization>>>)
                           -> Result<LTResult<D>>
        where NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Self::Serialization>>,
              ST: StateTransferMessage + 'static,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization>;

    /// Handle a timeout being received from the timeout layer
    fn handle_timeout<ST>(&mut self, timeout: Vec<RqTimeout>) -> Result<LTTimeoutResult>
        where ST: StateTransferMessage + 'static,
              NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Self::Serialization>>,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization>;
}


impl<D: ApplicationData> Debug for LTResult<D> {
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
                write!(f, "LTPFinished({:?}, {:?})", first, last)
            }
        }
    }
}