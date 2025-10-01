use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::error;

use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::exhaust_and_consume;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::{channel, threadpool, unwrap_channel};
use atlas_communication::message::StoredMessage;
use atlas_communication::stub::RegularNetworkStub;
use atlas_core::ordering_protocol::networking::serialize::NetworkView;
use atlas_core::timeouts::timeout::TimeoutModHandle;
use atlas_metrics::metrics::metric_duration;
use atlas_smr_application::state::monolithic_state::{
    digest_state, AppStateMessage, InstallStateMessage, MonolithicState,
};
use atlas_smr_core::persistent_log::MonolithicStateLog;
use atlas_smr_core::serialize::StateSys;
use atlas_smr_core::state_transfer::monolithic_state::{
    MonolithicStateTransfer, MonolithicStateTransferInitializer,
};
use atlas_smr_core::state_transfer::networking::StateTransferSendNode;
use atlas_smr_core::state_transfer::{Checkpoint, CstM};

use crate::metric::{APP_STATE_DIGEST_TIME_ID, STATE_TRANSFER_PROCESS_TIME_ID};
use crate::server::state_transfer::{
    StateTransferMngr, StateTransferThreadInnerHandle, StateTransferWorkMessage,
};
use crate::server::IterableProtocolRes;

pub struct MonStateTransfer<V, S, NT, PL, ST>
where
    V: NetworkView,
    S: MonolithicState + 'static,
    ST: MonolithicStateTransfer<S>,
    PL: MonolithicStateLog<S>,
{
    inner_state: StateTransferMngr<V, S, NT, PL, ST>,
    // Receiver of checkpoints from the application
    checkpoint_rx_from_app: ChannelSyncRx<AppStateMessage<S>>,

    digested_state: (
        ChannelSyncTx<Arc<ReadOnly<Checkpoint<S>>>>,
        ChannelSyncRx<Arc<ReadOnly<Checkpoint<S>>>>,
    ),

    state_transfer_protocol: ST,
}

impl<V, S, NT, PL, ST> MonStateTransfer<V, S, NT, PL, ST>
where
    V: NetworkView + 'static,
    S: MonolithicState + Send + 'static,
    ST: MonolithicStateTransfer<S> + 'static,
    PL: MonolithicStateLog<S> + 'static,
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
        ST: MonolithicStateTransferInitializer<S, NT, PL>,
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

                let digest_app_state =
                    channel::sync::new_bounded_sync(5, Some("Digested App State Channel"));

                let state_transfer_protocol =
                    ST::initialize(st_config, timeouts, node, persistent_log, state_tx.clone())
                        .expect("Failed to init state transfer protocol");

                let mut state_transfer_manager = Self {
                    inner_state: inner_mngr,
                    checkpoint_rx_from_app: checkpoint_rx,
                    digested_state: digest_app_state,
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
                IterableProtocolRes::ReRun => {
                    metric_duration(STATE_TRANSFER_PROCESS_TIME_ID, last_loop.elapsed());
                }
                IterableProtocolRes::Receive | IterableProtocolRes::Continue => {
                    metric_duration(STATE_TRANSFER_PROCESS_TIME_ID, last_loop.elapsed());

                    self.receive_from_all_channels()?;
                }
            };

            last_loop = Instant::now();
        }
    }

    fn receive_from_all_channels(&mut self) -> Result<()> {
        self.receive_from_all_channels_select()
    }

    fn receive_from_all_channels_select(&mut self) -> Result<()> {
        let inner_work_rx = self.inner_state.handle().work_rx().clone();

        channel::sync::sync_select_biased! {
            recv(unwrap_channel!(inner_work_rx)) -> work_msg => exhaust_and_consume!(work_msg?, inner_work_rx, self, handle_work_message),
            recv(unwrap_channel!(self.checkpoint_rx_from_app)) -> checkpoint_from_app => exhaust_and_consume!(checkpoint_from_app?, self.checkpoint_rx_from_app, self, handle_checkpoint_received),
            recv(unwrap_channel!(self.digested_state.1)) -> digested => exhaust_and_consume!(digested?, self.digested_state.1, self, handle_received_digested_checkpoint),
            recv(unwrap_channel!(self.inner_state.node().incoming_stub().as_ref())) -> network_msg =>
            exhaust_and_consume!(network_msg?, self.inner_state.node().incoming_stub().as_ref(), self, handle_network_message),
            default(Duration::from_millis(5)) => Ok(()),
        }
    }

    fn receive_from_all_channels_exhaust(&mut self) -> Result<()> {
        exhaust_and_consume!(
            self.inner_state.handle().work_rx(),
            self,
            handle_work_message
        );
        exhaust_and_consume!(
            self.checkpoint_rx_from_app,
            self,
            handle_checkpoint_received
        );
        exhaust_and_consume!(
            self.digested_state.1,
            self,
            handle_received_digested_checkpoint
        );
        exhaust_and_consume!(
            self.inner_state.node().incoming_stub().as_ref(),
            self,
            handle_network_message
        );

        Ok(())
    }

    fn handle_network_message(
        &mut self,
        message: StoredMessage<CstM<ST::Serialization>>,
    ) -> Result<()> {
        self.inner_state
            .handle_network_message(&mut self.state_transfer_protocol, message)
    }

    fn handle_work_message(&mut self, message: StateTransferWorkMessage<V>) -> Result<()> {
        self.inner_state
            .handle_work_message(&mut self.state_transfer_protocol, message)
    }

    fn handle_checkpoint_received(&mut self, checkpoint: AppStateMessage<S>) -> Result<()> {
        self.execution_finished_with_appstate(checkpoint.seq(), checkpoint.into_state())
    }

    fn handle_received_digested_checkpoint(
        &mut self,
        checkpoint: Arc<ReadOnly<Checkpoint<S>>>,
    ) -> Result<()> {
        self.state_transfer_protocol
            .handle_state_received_from_app(checkpoint.clone())?;
        self.inner_state
            .notify_of_checkpoint(checkpoint.sequence_number());
        Ok(())
    }

    /// handle the execution being finished with the app state
    fn execution_finished_with_appstate(&mut self, seq: SeqNo, appstate: S) -> Result<()> {
        // Get a handle to the digested state return channel
        let return_tx = self.digested_state.0.clone();

        // Digest the app state before passing it on to the ordering protocols
        threadpool::execute(move || {
            let start = Instant::now();

            let result = digest_state(&appstate);

            match result {
                Ok(digest) => {
                    let checkpoint = Checkpoint::new(seq, appstate, digest);

                    return_tx.send(checkpoint).unwrap();

                    metric_duration(APP_STATE_DIGEST_TIME_ID, start.elapsed());
                }
                Err(error) => {
                    error!(
                        "Failed to serialize and digest application state: {:?}",
                        error
                    )
                }
            }
        });

        Ok(())
    }
}
