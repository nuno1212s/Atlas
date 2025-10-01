use std::sync::Arc;
use std::time::Instant;

use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::exhaust_and_consume;
use atlas_communication::stub::RegularNetworkStub;
use atlas_core::ordering_protocol::networking::serialize::NetworkView;
use atlas_core::timeouts::timeout::TimeoutModHandle;
use atlas_metrics::metrics::metric_duration;
use atlas_smr_application::state::divisible_state::{
    AppState, AppStateMessage, DivisibleState, InstallStateMessage,
};
use atlas_smr_core::persistent_log::DivisibleStateLog;
use atlas_smr_core::serialize::StateSys;
use atlas_smr_core::state_transfer::divisible_state::{
    DivisibleStateTransfer, DivisibleStateTransferInitializer,
};
use atlas_smr_core::state_transfer::networking::StateTransferSendNode;
use tracing::error;

use crate::metric::STATE_TRANSFER_PROCESS_TIME_ID;
use crate::server::state_transfer::{StateTransferMngr, StateTransferThreadInnerHandle};
use crate::server::IterableProtocolRes;

pub struct DivStateTransfer<V, S, NT, PL, ST>
where
    V: NetworkView,
    S: DivisibleState + 'static,
    ST: DivisibleStateTransfer<S>,
    PL: DivisibleStateLog<S>,
{
    inner_state: StateTransferMngr<V, S, NT, PL, ST>,

    state_tx_to_executor: ChannelSyncTx<InstallStateMessage<S>>,
    checkpoint_rx_from_app: ChannelSyncRx<AppStateMessage<S>>,

    state_transfer_protocol: ST,
}

impl<V, S, NT, PL, ST> DivStateTransfer<V, S, NT, PL, ST>
where
    V: NetworkView + 'static,
    S: DivisibleState + Send + 'static,
    ST: DivisibleStateTransfer<S> + 'static,
    PL: DivisibleStateLog<S> + 'static,
    NT: StateTransferSendNode<ST::Serialization> + RegularNetworkStub<StateSys<ST::Serialization>>,
{
    pub fn init_state_transfer_thread(
        state_tx: ChannelSyncTx<InstallStateMessage<S>>,
        checkpoint_rx: ChannelSyncRx<AppStateMessage<S>>,
        st_config: ST::Config,
        node: Arc<NT>,
        timeouts: TimeoutModHandle,
        persistent_log: PL,
        handle: StateTransferThreadInnerHandle<V>,
        view: V,
    ) where
        ST: DivisibleStateTransferInitializer<S, NT, PL>,
        NT: StateTransferSendNode<ST::Serialization>
            + RegularNetworkStub<StateSys<ST::Serialization>>
            + 'static,
    {
        std::thread::Builder::new()
            .name(String::from("State transfer thread"))
            .spawn(move || {
                let inner_mngr =
                    StateTransferMngr::initialize_core_state_transfer(node.clone(), handle, view)
                        .expect("Failed to initialize state transfer inner layer");

                let state_transfer_protocol =
                    ST::initialize(st_config, timeouts, node, persistent_log, state_tx.clone())
                        .expect("Failed to init state transfer protocol");

                let mut state_transfer_manager = Self {
                    inner_state: inner_mngr,
                    state_tx_to_executor: state_tx,
                    checkpoint_rx_from_app: checkpoint_rx,
                    state_transfer_protocol,
                };

                loop {
                    if let Err(err) = state_transfer_manager.run() {
                        error!("Received state transfer error {:?}", err);
                    }
                }
            })
            .expect("Failed to allocate the state transfer thread");
    }

    pub fn run(&mut self) -> Result<()> {
        let mut last_loop = Instant::now();

        loop {
            match self
                .inner_state
                .iterate(&mut self.state_transfer_protocol)?
            {
                IterableProtocolRes::ReRun => {}
                IterableProtocolRes::Receive | IterableProtocolRes::Continue => {
                    metric_duration(STATE_TRANSFER_PROCESS_TIME_ID, last_loop.elapsed());

                    self.receive_from_all_channels()?;
                }
            };

            last_loop = Instant::now();
        }
    }

    fn receive_from_all_channels(&mut self) -> Result<()> {
        self.receive_from_all_channels_exhaust()
    }

    /*fn receive_from_all_channels_select(&mut self) -> Result<()> {
        let inner_handle = self.inner_state.handle();

        channel::sync::sync_select_biased! {
            recv(unwrap_channel!(inner_handle.work_rx())) -> work_msg =>
            self.inner_state.handle_work_message(&mut self.state_transfer_protocol, work_msg?),
            recv(unwrap_channel!(self.checkpoint_rx_from_app)) -> checkpoint_msg =>
            self.handle_checkpoint_message(checkpoint_msg?),
            recv(unwrap_channel!(self.inner_state.node().incoming_stub().as_ref())) -> network_msg =>
            self.inner_state.handle_network_message(&mut self.state_transfer_protocol, network_msg?),
        }
    }*/

    fn receive_from_all_channels_exhaust(&mut self) -> Result<()> {
        let inner_handle = self.inner_state.handle().clone();

        while let Ok(message) = inner_handle.work_rx().try_recv() {
            self.inner_state
                .handle_work_message(&mut self.state_transfer_protocol, message)?;
        }

        exhaust_and_consume!(self.checkpoint_rx_from_app, self, handle_checkpoint_message);

        while let Ok(message) = self.inner_state.node().incoming_stub().as_ref().try_recv() {
            self.inner_state
                .handle_network_message(&mut self.state_transfer_protocol, message)?;
        }

        Ok(())
    }

    fn handle_checkpoint_message(&mut self, checkpoint: AppStateMessage<S>) -> Result<()> {
        let (seq_no, state) = checkpoint.into_state();

        match state {
            AppState::StateDescriptor(descriptor) => {
                self.state_transfer_protocol
                    .handle_state_desc_received_from_app(descriptor)?;
            }
            AppState::StatePart(parts) => {
                self.state_transfer_protocol
                    .handle_state_part_received_from_app(parts.into_vec())?;
            }
            AppState::Done => {
                self.state_transfer_protocol
                    .handle_state_finished_reception()?;
                self.inner_state.notify_of_checkpoint(seq_no);
            }
        }

        Ok(())
    }
}
