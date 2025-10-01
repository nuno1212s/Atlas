#![feature(btree_extract_if)]
#![allow(dead_code)]

extern crate core;

use anyhow::Context;
use getset::Getters;
use lazy_static::lazy_static;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, trace, warn};

use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::circuit_breaker::CircuitBreaker;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::{channel, exhaust_and_consume, unwrap_channel};
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration::{
    NetworkInformationProvider, NetworkUpdatedMessage, ReconfigurationNetworkCommunication,
};
use atlas_communication::stub::RegularNetworkStub;
use atlas_core::reconfiguration_protocol::{
    QuorumJoinCert, ReconfigResponse, ReconfigurationCommunicationHandles, ReconfigurationProtocol,
};
use atlas_core::timeouts::timeout::{ModTimeout, TimeoutModHandle, TimeoutableMod};
use atlas_core::timeouts::TimeoutID;

use crate::config::ReconfigurableNetworkConfig;
use crate::message::{ReconfData, ReconfigMessage, ReconfigurationMessage};
use crate::network_reconfig::{GeneralNodeInfo, NetworkInfo, NetworkNodeState};
use crate::quorum_config::network::QuorumConfigNetworkWrapper;
use crate::quorum_config::{Node, QuorumObserver, QuorumView};

pub mod config;
pub mod message;
mod metrics;
pub mod network_reconfig;
pub mod quorum_config;
//pub mod threshold_crypto;

const TIMEOUT_DUR: Duration = Duration::from_secs(3);

lazy_static! {
    static ref MOD_NAME: Arc<str> = Arc::from("RECONFIG");
    static ref MESSAGE_SLEEP: Duration = Duration::from_millis(1000);
}

/// The reconfiguration module.
/// Provides various utilities for allowing reconfiguration of the network
/// Such as message definitions, important types and etc.
///
/// This module will then be used by the parts of the system which must be reconfigurable
/// (For example, the network node, the client node, etc)
#[derive(Debug)]
enum ReconfigurableNodeState {
    NetworkReconfigurationProtocol,
    QuorumReconfigurationProtocol,
    Stable,
}

/// The response returned from iterating the network protocol
pub enum NetworkProtocolResponse {
    Done,
    /// Just a response to indicate nothing was done
    Nil,
}

/// The response returned from iterating the quorum protocol
pub enum QuorumProtocolResponse {
    DoneInitialSetup,
    UpdatedQuorum(QuorumView),
    Nil,
}

/// A reconfigurable node, used to handle the reconfiguration of the network as a whole
#[derive(Getters)]
pub struct ReconfigurableNode<NT>
where
    NT: Send + 'static,
{
    seq_gen: SeqNoGen,
    /// The reconfigurable node state
    node_state: ReconfigurableNodeState,
    /// The general information about the network
    node: GeneralNodeInfo,
    /// The reference to the network node
    network_node: Arc<NT>,
    /// The message handler to send network updates to the communication layer
    #[get = "pub(crate)"]
    reconfig_network: ReconfigurationNetworkCommunication,
    /// Handle to the timeouts module
    timeouts: TimeoutModHandle,
    // Receive messages from the other protocols
    channel_rx: ChannelSyncRx<ReconfigMessage>,
    /// The type of the node we are running.
    node_type: Node,
}

impl<NT> ReconfigurableNode<NT>
where
    NT: Send + 'static,
{
    fn switch_state(&mut self, new_state: ReconfigurableNodeState) {
        match (&self.node_state, &new_state) {
            (
                ReconfigurableNodeState::NetworkReconfigurationProtocol,
                ReconfigurableNodeState::QuorumReconfigurationProtocol,
            ) => {
                warn!("We have finished the network reconfiguration protocol, running the quorum reconfiguration message");
            }
            (
                ReconfigurableNodeState::QuorumReconfigurationProtocol,
                ReconfigurableNodeState::Stable,
            ) => {
                warn!("We have finished the quorum reconfiguration protocol, switching to stable.");
            }
            (_, _) => {
                warn!(
                    "Illegal transition of states, {:?} to {:?}",
                    self.node_state, new_state
                );

                return;
            }
        }

        self.node_state = new_state;
    }

    fn run(&mut self) -> Result<()>
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        loop {
            trace!(
                "Iterating the reconfiguration protocol, current state {:?}",
                self.node_state
            );

            match self.node_state {
                ReconfigurableNodeState::NetworkReconfigurationProtocol => {
                    match self.node.iterate(
                        &mut self.seq_gen,
                        &self.network_node,
                        &self.timeouts,
                    )? {
                        NetworkProtocolResponse::Done => {
                            self.switch_state(
                                ReconfigurableNodeState::QuorumReconfigurationProtocol,
                            );
                        }
                        NetworkProtocolResponse::Nil => {}
                    };
                }
                ReconfigurableNodeState::QuorumReconfigurationProtocol => {
                    let node_wrap = QuorumConfigNetworkWrapper::from(self.network_node.clone());

                    match self.node_type.iterate(&node_wrap)? {
                        QuorumProtocolResponse::DoneInitialSetup => {
                            self.switch_state(ReconfigurableNodeState::Stable);
                        }
                        QuorumProtocolResponse::UpdatedQuorum(_) => {}
                        QuorumProtocolResponse::Nil => {}
                    }
                }
                ReconfigurableNodeState::Stable => {
                    let node_wrap = QuorumConfigNetworkWrapper::from(self.network_node.clone());

                    // We still want to iterate the quorum protocol in order to receive new updates from the ordering protocol
                    // The network reconfiguration protocol is now only request based, so it does not need to be iterated
                    self.node_type.iterate(&node_wrap)?;
                }
            }

            self.receive_from_incoming_channels()?;
        }
    }

    fn receive_from_incoming_channels(&mut self) -> Result<()>
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        self.receive_from_incoming_channels_select()
    }

    fn receive_from_incoming_channels_select(&mut self) -> Result<()>
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        channel::sync::sync_select_biased! {
            recv(unwrap_channel!(self.channel_rx)) -> orchestrator_message => {
                self.handle_message_from_orchestrator(orchestrator_message.context("Orchestrator message")?)
            }
            recv(unwrap_channel!(self.reconfig_network.network_update_receiver())) -> network_update_message => {
                self.handle_network_update_message(network_update_message.context("Update message")?)
            }
            recv(unwrap_channel!(self.network_node.incoming_stub().as_ref())) -> network_msg => {
                exhaust_and_consume!(
                    network_msg.context("Network stub receive")?,
                    self.network_node.incoming_stub().as_ref(),
                    self,
                    handle_network_message
                )
            }
            default(*MESSAGE_SLEEP) => Ok(())
        }
    }

    fn receive_from_incoming_channels_exhaust(&mut self) -> Result<()>
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        exhaust_and_consume!(self.channel_rx, self, handle_message_from_orchestrator);

        exhaust_and_consume!(
            self.reconfig_network.network_update_receiver(),
            self,
            handle_network_update_message
        );

        exhaust_and_consume!(
            self.network_node.incoming_stub().as_ref(),
            self,
            handle_network_message
        );

        Ok(())
    }

    fn handle_network_message(
        &mut self,
        network_message: StoredMessage<ReconfigurationMessage>,
    ) -> Result<()>
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        let (header, message): (Header, ReconfigurationMessage) = network_message.into_inner();

        match message {
            ReconfigurationMessage::NetworkReconfig(network_reconfig) => {
                if self.node.is_response_to_request(
                    &self.seq_gen,
                    &header,
                    network_reconfig.sequence_number(),
                    network_reconfig.message_type(),
                ) {
                    self.timeouts.ack_received(
                        TimeoutID::SeqNoBased(network_reconfig.sequence_number()),
                        header.from(),
                    )?;
                }

                match self.node.handle_network_reconfig_msg(
                    &mut self.seq_gen,
                    &self.network_node,
                    &self.reconfig_network,
                    &self.timeouts,
                    header,
                    network_reconfig,
                ) {
                    NetworkProtocolResponse::Done => {
                        self.switch_state(ReconfigurableNodeState::QuorumReconfigurationProtocol);
                    }
                    NetworkProtocolResponse::Nil => {}
                };
            }
            ReconfigurationMessage::QuorumConfig(quorum_msg) => {
                //TODO: Handle timeouts

                let node_wrap = QuorumConfigNetworkWrapper::from(self.network_node.clone());

                match self
                    .node_type
                    .handle_message(&node_wrap, header, quorum_msg)?
                {
                    QuorumProtocolResponse::DoneInitialSetup => {
                        debug!("We have finished the initial setup of the quorum protocol, switching to stable");

                        self.switch_state(ReconfigurableNodeState::Stable);
                    }
                    QuorumProtocolResponse::UpdatedQuorum(_) => {}
                    QuorumProtocolResponse::Nil => {}
                }
            }
            ReconfigurationMessage::ThresholdCrypto(_) => {}
        }

        Ok(())
    }

    fn handle_message_from_orchestrator(&mut self, reconfig_message: ReconfigMessage) -> Result<()>
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        match reconfig_message {
            ReconfigMessage::TimeoutReceived(timeout) => {
                info!("We have received timeouts {:?}", timeout);

                for rq_timeout in timeout {
                    let seq = match rq_timeout.id() {
                        TimeoutID::SeqNoBased(seq) => *seq,
                        _ => unreachable!(),
                    };

                    match self.node_state {
                        ReconfigurableNodeState::NetworkReconfigurationProtocol => {
                            if seq != self.seq_gen.curr_seq() {
                                error!("Received a reconfiguration timeout with a different sequence number than the current one {:?} != {:?}",
                                                   seq, self.seq_gen.curr_seq());

                                continue;
                            }

                            self.node.handle_timeout(
                                &mut self.seq_gen,
                                &self.network_node,
                                &self.timeouts,
                            );
                        }
                        ReconfigurableNodeState::QuorumReconfigurationProtocol => {
                            let nt_wrap =
                                QuorumConfigNetworkWrapper::from(self.network_node.clone());

                            self.node_type.handle_timeout(&nt_wrap, &self.timeouts);
                        }
                        ReconfigurableNodeState::Stable => {
                            error!("Received a reconfiguration timeout while we are stable, this does not make sense");
                        }
                    }

                    if let Err(err) = self.timeouts.cancel_timeout(TimeoutID::SeqNoBased(seq)) {
                        error!("Error while cancelling the timeout: {:?}", err);
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_network_update_message(&mut self, _network_msg: NetworkUpdatedMessage) -> Result<()> {
        todo!()
    }
}

#[derive(Debug)]
struct SeqNoGen {
    seq: SeqNo,
}

impl SeqNoGen {
    pub fn curr_seq(&self) -> SeqNo {
        self.seq
    }

    pub fn next_seq(&mut self) -> SeqNo {
        self.seq += SeqNo::ONE;

        self.seq
    }
}

/// The handle to the current reconfigurable node information.
///
pub struct ReconfigurableNodeProtocolHandle {
    network_info: Arc<NetworkInfo>,
    quorum_info: QuorumObserver,
    channel_tx: ChannelSyncTx<ReconfigMessage>,
}

impl TimeoutableMod<ReconfigResponse> for ReconfigurableNodeProtocolHandle {
    fn mod_name() -> Arc<str> {
        MOD_NAME.clone()
    }

    fn handle_timeout(&mut self, timeout: Vec<ModTimeout>) -> Result<ReconfigResponse> {
        self.handle_timeouts_safe(timeout)
    }
}

impl ReconfigurationProtocol for ReconfigurableNodeProtocolHandle {
    type Config = ReconfigurableNetworkConfig;
    type InformationProvider = NetworkInfo;
    type Serialization = ReconfData;

    fn init_default_information(config: Self::Config) -> Result<Arc<Self::InformationProvider>> {
        Ok(Arc::new(NetworkInfo::init_from_config(config)))
    }

    async fn initialize_protocol<NT>(
        information: Arc<Self::InformationProvider>,
        node: Arc<NT>,
        timeouts: TimeoutModHandle,
        comm_handle: ReconfigurationCommunicationHandles,
        network_updater: ReconfigurationNetworkCommunication,
        _min_stable_node_count: usize,
    ) -> Result<Self>
    where
        NT: RegularNetworkStub<Self::Serialization> + 'static,
        Self: Sized,
    {
        let (network_update_handle, node_type) = comm_handle.into();

        let general_info = GeneralNodeInfo::new(
            information.clone(),
            NetworkNodeState::Init,
            network_update_handle,
        );

        let quorum_view = Node::init_observer(information.bootstrap_nodes().clone());

        let our_info = information.own_node_info().clone();

        let (channel_tx, channel_rx) =
            channel::sync::new_bounded_sync(128, Some("Reconfiguration message channel"));

        let cpy_obs = quorum_view.clone();

        std::thread::Builder::new()
            .name("Reconfiguration Protocol Thread".to_string())
            .spawn(move || {
                let node_type = Node::initialize_with_observer(our_info, cpy_obs, node_type);

                let mut reconfigurable_node = ReconfigurableNode {
                    seq_gen: SeqNoGen { seq: SeqNo::ZERO },
                    node_state: ReconfigurableNodeState::NetworkReconfigurationProtocol,
                    node: general_info,
                    network_node: node.clone(),
                    reconfig_network: network_updater,
                    timeouts,
                    channel_rx,
                    node_type,
                };

                if let Err(err) = CircuitBreaker::execute_in_circuit_breaker(|| reconfigurable_node.run(), None) {
                    error!(
                        "Consistent error while running the reconfiguration protocol: {:?}. Terminating thread.",
                        err
                    );
                }
            })
            .expect("Failed to launch reconfiguration protocol thread");

        let node_handle = ReconfigurableNodeProtocolHandle {
            network_info: information.clone(),
            quorum_info: quorum_view,
            channel_tx,
        };

        Ok(node_handle)
    }

    fn get_quorum_members(&self) -> Vec<NodeId> {
        self.quorum_info.current_view().quorum_members().clone()
    }

    fn get_current_f(&self) -> usize {
        self.quorum_info.current_view().f()
    }

    fn quorum_state(&self) -> (Vec<NodeId>, usize) {
        let view = self.quorum_info.current_view();

        (view.quorum_members().clone(), view.f())
    }

    fn is_join_certificate_valid(
        &self,
        _certificate: &QuorumJoinCert<Self::Serialization>,
    ) -> bool {
        //TODO: Analyse the veracity of this join certificate according to the information we have on the
        // current quorum
        true
    }

    fn handle_timeouts_safe(&self, timeouts: Vec<ModTimeout>) -> Result<ReconfigResponse> {
        self.channel_tx
            .send(ReconfigMessage::TimeoutReceived(timeouts))?;

        Ok(ReconfigResponse::Running)
    }
}

/// The result of the iteration of the node
#[derive(Clone, Debug)]
enum IterationResult {
    ReceiveMessage,
    Idle,
}
