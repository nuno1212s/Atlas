use atlas_communication::Node;
use atlas_common::error::*;
use atlas_execution::serialize::SharedData;
use atlas_execution::state::divisible_state::DivisibleState;
use crate::persistent_log::DivisibleStateLog;
use crate::serialize::{LogTransferMessage, OrderingProtocolMessage, ServiceMsg};
use crate::state_transfer::StateTransferPT;

pub trait DivisibleStateTransfer<S, NT, PL>: StateTransferPT<S, NT, PL> where S: DivisibleState + 'static,
                                                                              PL: DivisibleStateLog<S> {
    
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