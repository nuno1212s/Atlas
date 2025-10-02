use std::sync::Arc;

use atlas_common::channel::sync::ChannelSyncTx;
use atlas_common::error::*;
use atlas_core::timeouts::timeout::TimeoutModHandle;
use atlas_smr_application::state::divisible_state::{DivisibleState, InstallStateMessage};

use crate::persistent_log::DivisibleStateLog;
use crate::state_transfer::networking::StateTransferSendNode;
use crate::state_transfer::StateTransferProtocol;

pub trait DivisibleStateTransfer<S>: StateTransferProtocol<S>
where
    S: DivisibleState + 'static,
{
    /// The configuration type the state transfer protocol wants to accept
    type Config: Send;

    /// Handle having received a state from the application
    fn handle_state_desc_received_from_app(&mut self, descriptor: S::StateDescriptor)
        -> Result<()>;

    fn handle_state_part_received_from_app(&mut self, parts: Vec<S::StatePart>) -> Result<()>;

    /// Handle the state being finished
    fn handle_state_finished_reception(&mut self) -> Result<()>;
}

pub trait DivisibleStateTransferInitializer<S, NT, PL>: DivisibleStateTransfer<S>
where
    S: DivisibleState + 'static,
{
    /// Initialize the state transferring protocol with the given configuration, timeouts and communication layer
    fn initialize(
        config: Self::Config,
        timeouts: TimeoutModHandle,
        node: Arc<NT>,
        log: PL,
        executor_state_handle: ChannelSyncTx<InstallStateMessage<S>>,
    ) -> Result<Self>
    where
        Self: Sized,
        NT: StateTransferSendNode<Self::Serialization>,
        PL: DivisibleStateLog<S>;
}
