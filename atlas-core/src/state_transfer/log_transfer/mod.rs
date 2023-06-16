use std::sync::Arc;
use atlas_common::error::*;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::StoredMessage;
use atlas_communication::Node;
use atlas_execution::serialize::SharedData;
use crate::messages::StateTransfer;
use crate::ordering_protocol::{OrderingProtocol, OrderingProtocolArgs, View};
use crate::persistent_log::{StatefulOrderingProtocolLog, StateTransferProtocolLog};
use crate::serialize::{LogTransferMessage, ServiceMsg, StatefulOrderProtocolMessage};
use crate::timeouts::Timeouts;

pub type DecLog<OP> = <OP as StatefulOrderProtocolMessage>::DecLog;

pub type LogTM<M: LogTransferMessage> = <M as LogTransferMessage>::LogTransferMessage;

/// An order protocol that uses the log transfer protocol to manage its log
pub trait StatefulOrderProtocol<D: SharedData + 'static, NT, PL>: OrderingProtocol<D, NT, PL> {

    /// The serialization abstraction for ordering protocols with logs, so we can then send it across the network
    type StateSerialization: StatefulOrderProtocolMessage + 'static;

    fn initialize_with_initial_state(config: Self::Config, args: OrderingProtocolArgs<D, NT, PL>,
                                     dec_log: DecLog<Self::StateSerialization>) -> Result<Self> where
        Self: Sized;

    /// Install a state received from other replicas in the system
    /// Should only alter the necessary things within its own state and
    /// then should return the state and a list of all requests that should
    /// then be executed by the application.
    fn install_state(&mut self, view_info: View<Self::Serialization>,
                     dec_log: DecLog<Self::StateSerialization>) -> Result<Vec<D::Request>>
        where PL: StatefulOrderingProtocolLog<Self::Serialization, Self::StateSerialization>;

    /// Snapshot the current log of the replica
    fn snapshot_log(&mut self) -> Result<(View<Self::Serialization>, DecLog<Self::StateSerialization>)>
        where PL: StatefulOrderingProtocolLog<Self::Serialization, Self::StateSerialization>;

    /// Notify the consensus protocol that we have a checkpoint at a given sequence number,
    /// meaning it can now be garbage collected
    fn checkpointed(&mut self, seq: SeqNo) -> Result<()>
        where PL: StatefulOrderingProtocolLog<Self::Serialization, Self::StateSerialization>;
}

/// The result of processing a message in the log transfer protocol
pub enum LTResult<D: SharedData> {
    RunLTP,
    NotNeeded,
    Running,
    /// The log transfer protocol has finished and the ordering protocol should now
    /// be proceeded. The requests contained are requests that must be executed by the application
    /// in order to reach the state that corresponds to the decision log
    LTPFinished(Vec<D::Request>),
}


pub trait LogTransferProtocol<D, OP, NT, PL> where D: SharedData + 'static, OP: StatefulOrderProtocol<D, NT, PL> + 'static {

    /// The type which implements StateTransferMessage, to be implemented by the developer
    type Serialization: LogTransferMessage + 'static;

    /// The configuration type the protocol wants to accept
    type Config;

    /// Initialize the state transferring protocol with the given configuration, timeouts and communication layer
    fn initialize(config: Self::Config, timeouts: Timeouts, node: Arc<NT>, log: PL) -> Result<Self>
        where Self: Sized;

    /// Request the latest state from the rest of replicas
    fn request_latest_state(&mut self,
                            order_protocol: &mut OP) -> Result<()>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization>;

    /// Handle a state transfer protocol message that was received while executing the ordering protocol
    fn handle_off_ctx_message(&mut self,
                              order_protocol: &mut OP,
                              message: StoredMessage<StateTransfer<LogTM<Self::Serialization>>>)
                              -> Result<()>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>,
              PL:StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization>;

    /// Process a state transfer protocol message, received from other replicas
    /// We also provide a mutable reference to the stateful ordering protocol, so the
    /// state can be installed (if that's the case)
    fn process_message(&mut self,
                       order_protocol: &mut OP,
                       message: StoredMessage<StateTransfer<LogTM<Self::Serialization>>>)
                       -> Result<LTResult<D>>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization>;
}