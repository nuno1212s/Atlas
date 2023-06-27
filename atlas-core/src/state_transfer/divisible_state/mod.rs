use std::sync::Arc;
use atlas_common::channel::ChannelSyncTx;
use atlas_communication::Node;
use atlas_common::error::*;
use atlas_execution::serialize::ApplicationData;
use atlas_execution::state::divisible_state::{DivisibleState, InstallStateMessage};
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
    fn initialize(config: Self::Config, timeouts: Timeouts, node: Arc<NT>, log: PL,
                  executor_state_handle: ChannelSyncTx<InstallStateMessage<S>>) -> Result<Self>
        where Self: Sized;

    /// Handle having received a state from the application
    /// you should also notify the ordering protocol that the state has been received
    /// and processed, so he is now safe to delete the state (Maybe this should be handled by the replica?)
    fn handle_state_received_from_app<D, OP, LP>(&mut self,
                                                 descriptor: S::StateDescriptor,
                                                 state: Vec<S::StatePart>) -> Result<()>
        where D: ApplicationData + 'static,
              OP: OrderingProtocolMessage + 'static,
              LP: LogTransferMessage + 'static,
              NT: Node<ServiceMsg<D, OP, Self::Serialization, LP>>;
}