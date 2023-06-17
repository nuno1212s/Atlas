use std::sync::Arc;
use atlas_communication::Node;
use atlas_common::error::*;
use atlas_execution::serialize::SharedData;
use atlas_execution::state::divisible_state::DivisibleState;
use crate::persistent_log::DivisibleStateLog;
use crate::serialize::{LogTransferMessage, OrderingProtocolMessage, ServiceMsg};
use crate::state_transfer::StateTransferProtocol;
use crate::timeouts::Timeouts;

pub trait DivisibleStateTransfer<S, NT, PL>: StateTransferProtocol<S, NT, PL>
    where S: DivisibleState + 'static,
          PL: DivisibleStateLog<S> {
    /// The configuration type the state transfer protocol wants to accept
    type Config;

    /// Initialize the state transferring protocol with the given configuration, timeouts and communication layer
    fn initialize(config: Self::Config, timeouts: Timeouts, node: Arc<NT>, log: PL) -> Result<Self>
        where Self: Sized;

    /// Handle having received a state from the application
    /// you should also notify the ordering protocol that the state has been received
    /// and processed, so he is now safe to delete the state (Maybe this should be handled by the replica?)
    fn handle_state_received_from_app<D, OP, LP>(&mut self,
                                                 descriptor: Vec<S::StateDescriptor>,
                                                 state: Vec<S::StatePart>) -> Result<()>
        where D: SharedData + 'static,
              OP: OrderingProtocolMessage,
              LP: LogTransferMessage,
              NT: Node<ServiceMsg<D, OP, Self::Serialization, LP>>;
}