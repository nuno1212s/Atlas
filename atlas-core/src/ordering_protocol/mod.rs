use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use atlas_communication::message::{Header, NetworkMessage, StoredMessage, System};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::Orderable;
use atlas_communication::Node;
use atlas_execution::ExecutorHandle;
use atlas_execution::serialize::SharedData;
use crate::messages::{ForwardedRequestsMessage, Protocol, StoredRequestMessage, SystemMessage};
use crate::request_pre_processing::{BatchOutput, RequestPreProcessor};
use crate::serialize::{OrderingProtocolMessage, StateTransferMessage, ServiceMsg, NetworkView};
use crate::timeouts::{RqTimeout, Timeout, Timeouts};

pub type View<OP> = <OP as OrderingProtocolMessage>::ViewInfo;

pub type ProtocolMessage<OP> = <OP as OrderingProtocolMessage>::ProtocolMessage;

pub struct OrderingProtocolArgs<D, NT>(pub ExecutorHandle<D>, pub Timeouts, pub RequestPreProcessor<D::Request>, pub BatchOutput<D::Request>, pub Arc<NT>) where D: SharedData;

/// The trait for an ordering protocol to be implemented in Atlas
pub trait OrderingProtocol<D, NT>: Orderable where D: SharedData + 'static {

    /// The type which implements OrderingProtocolMessage, to be implemented by the developer
    type Serialization: OrderingProtocolMessage + 'static;

    /// The configuration type the protocol wants to accept
    type Config;

    /// Initialize this ordering protocol with the given configuration, executor, timeouts and node
    fn initialize(config: Self::Config, args: OrderingProtocolArgs<D, NT>) -> Result<Self> where
        Self: Sized;

    /// Get the current view of the ordering protocol
    fn view(&self) -> View<Self::Serialization>;

    /// Handle a protocol message that was received while we are executing another protocol
    fn handle_off_ctx_message(&mut self, message: StoredMessage<Protocol<ProtocolMessage<Self::Serialization>>>);

    /// Handle the protocol being executed having changed (for example to the state transfer protocol)
    /// This is important for some of the protocols, which need to know when they are being executed or not
    fn handle_execution_changed(&mut self, is_executing: bool) -> Result<()>;

    /// Poll from the ordering protocol in order to know what we should do next
    /// We do this to check if there are already messages waiting to be executed that were received ahead of time and stored.
    /// Or whether we should run state tranfer or wait for messages from other replicas
    fn poll(&mut self) -> OrderProtocolPoll<ProtocolMessage<Self::Serialization>>;

    /// Process a protocol message that we have received
    /// This can be a message received from the poll() method or a message received from other replicas.
    fn process_message(&mut self, message: StoredMessage<Protocol<ProtocolMessage<Self::Serialization>>>) -> Result<OrderProtocolExecResult>;

    /// Handle a timeout received from the timeouts layer
    fn handle_timeout(&mut self, timeout: Vec<RqTimeout>) -> Result<OrderProtocolExecResult>;
}

/// result from polling the ordering protocol
pub enum OrderProtocolPoll<P> {
    RunCst,
    ReceiveFromReplicas,
    Exec(StoredMessage<Protocol<P>>),
    RePoll,
}

pub enum OrderProtocolExecResult {
    Success,
    RunCst,
}

impl<P> Debug for OrderProtocolPoll<P> where P: Debug {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderProtocolPoll::RunCst => {
                write!(f, "RunCst")
            }
            OrderProtocolPoll::ReceiveFromReplicas => {
                write!(f, "Receive From Replicas")
            }
            OrderProtocolPoll::Exec(message) => {
                write!(f, "Exec message {:?}", message.message() )
            }
            OrderProtocolPoll::RePoll => {
                write!(f, "RePoll")
            }
        }
    }
}