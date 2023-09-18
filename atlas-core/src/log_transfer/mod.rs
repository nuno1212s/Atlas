use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use atlas_common::error::*;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::StoredMessage;
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_execution::serialize::ApplicationData;

use crate::log_transfer::networking::serialize::LogTransferMessage;
use crate::messages::LogTransfer;
use crate::ordering_protocol::OrderingProtocol;
use crate::ordering_protocol::stateful_order_protocol::StatefulOrderProtocol;
use crate::persistent_log::StatefulOrderingProtocolLog;
use crate::timeouts::{RqTimeout, Timeouts};

pub mod networking;

pub type LogTM<D, OP, M: LogTransferMessage<D, OP>> = <M as LogTransferMessage<D, OP>>::LogTransferMessage;

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
    type Serialization: LogTransferMessage<D, OP::Serialization> + 'static;

    /// The configuration type the protocol wants to accept
    type Config;

    /// Initialize the state transferring protocol with the given configuration, timeouts and communication layer
    fn initialize(config: Self::Config, timeouts: Timeouts, node: Arc<NT>, log: PL) -> Result<Self>
        where Self: Sized;

    /// Request the latest state from the rest of replicas
    fn request_latest_log(&mut self, order_protocol: &mut OP) -> Result<()>
        where PL: StatefulOrderingProtocolLog<D, OP::Serialization, OP::StateSerialization, OP::PermissionedSerialization>;

    /// Handle a state transfer protocol message that was received while executing the ordering protocol
    fn handle_off_ctx_message(&mut self, order_protocol: &mut OP,
                              message: StoredMessage<LogTransfer<LogTM<D, OP::Serialization, Self::Serialization>>>)
                              -> Result<()>
        where PL: StatefulOrderingProtocolLog<D, OP::Serialization, OP::StateSerialization, OP::PermissionedSerialization>;

    /// Process a state transfer protocol message, received from other replicas
    /// We also provide a mutable reference to the stateful ordering protocol, so the
    /// state can be installed (if that's the case)
    fn process_message(&mut self, order_protocol: &mut OP,
                       message: StoredMessage<LogTransfer<LogTM<D, OP::Serialization, Self::Serialization>>>)
                       -> Result<LTResult<D>>
        where PL: StatefulOrderingProtocolLog<D, OP::Serialization, OP::StateSerialization, OP::PermissionedSerialization>;

    /// Handle a timeout being received from the timeout layer
    fn handle_timeout(&mut self, timeout: Vec<RqTimeout>) -> Result<LTTimeoutResult>
        where PL: StatefulOrderingProtocolLog<D, OP::Serialization, OP::StateSerialization, OP::PermissionedSerialization>;
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