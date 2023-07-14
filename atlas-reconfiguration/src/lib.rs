#![feature(async_fn_in_trait)]

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use log::{error, info, warn};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use atlas_common::{async_runtime as rt, channel};
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::{Header, Reconfig};
use atlas_communication::NodeConnections;
use atlas_communication::reconfiguration_node::{ReconfigurationIncomingHandler, ReconfigurationNode};
use atlas_core::reconfiguration_protocol::{QuorumJoinCert, ReconfigResponse, ReconfigurableNodeTypes, ReconfigurationProtocol};
use atlas_core::timeouts::{RqTimeout, TimeoutKind, Timeouts};

use crate::config::ReconfigurableNetworkConfig;
use crate::message::{NetworkJoinResponseMessage, NetworkReconfigMessage, NetworkReconfigMsgType, NodeTriple, QuorumJoinCertificate, ReconfData, ReconfigMessage, ReconfigurationMessage};
use crate::network_reconfig::NetworkInfo;
use crate::node_types::client::ClientQuorumView;
use crate::node_types::NodeType;
use crate::node_types::replica::ReplicaQuorumView;
use crate::quorum_reconfig::QuorumView;

pub mod config;
pub mod message;
pub mod network_reconfig;
pub mod quorum_reconfig;
mod metrics;
mod node_types;

const TIMEOUT_DUR: Duration = Duration::from_secs(3);

/// The reconfiguration module.
/// Provides various utilities for allowing reconfiguration of the network
/// Such as message definitions, important types and etc.
///
/// This module will then be used by the parts of the system which must be reconfigurable
/// (For example, the network node, the client node, etc)

/// The reconfigurable node which will handle all reconfiguration requests
/// This handles the network level reconfiguration, not the quorum level reconfiguration
pub struct GeneralNodeInfo {
    curr_seq: SeqNo,
    /// Our current view of the network, including nodes we know about, their addresses and public keys
    network_view: Arc<NetworkInfo>,
    /// The current state of the network node, to keep track of which protocols we are executing
    current_state: NetworkNodeState,
}

/// A reconfigurable node, used to handle the reconfiguration of the network as a whole
pub struct ReconfigurableNode<NT> where NT: Send + 'static {
    /// The general information about the network
    node: GeneralNodeInfo,
    /// The reference to the network node
    network_node: Arc<NT>,
    /// Handle to the timeouts module
    timeouts: Timeouts,
    // Receive messages from the other protocols
    channel_rx: ChannelSyncRx<ReconfigMessage>,
    /// The type of the node we are running.
    node_type: NodeType<QuorumJoinCertificate>,
}

/// The handle to the current reconfigurable node information.
///
pub struct ReconfigurableNodeProtocol {
    network_info: Arc<NetworkInfo>,
    quorum_info: Arc<RwLock<QuorumView>>,
    channel_tx: ChannelSyncTx<ReconfigMessage>
}

/// The current state of our node (network level, not quorum level)
/// Quorum level operations will only take place when we are a stable member of the protocol
#[derive(Debug, Clone)]
pub enum NetworkNodeState {
    /// The node is currently initializing and will attempt to join the network
    Init,
    /// We have broadcast the join request to the known nodes and are waiting for their responses
    /// Which contain the network information. Afterwards, we will attempt to introduce ourselves to all
    /// nodes in the network (if there are more nodes than the known boostrap nodes)
    JoiningNetwork { contacted: usize, responded: BTreeSet<NodeId> },
    /// We are currently introducing ourselves to the network (and attempting to acquire all known nodes)
    IntroductionPhase { contacted: usize, responded: BTreeSet<NodeId> },
    /// A stable member of the network, up to date with the current membership
    StableMember,
    /// We are currently leaving the network
    LeavingNetwork,
}

/// The result of the iteration of the node
#[derive(Clone, Debug)]
enum IterationResult {
    ReceiveMessage,
    Idle,
}

impl GeneralNodeInfo {
    fn next_seq(&mut self) -> SeqNo {
        self.curr_seq += SeqNo::ONE;

        self.curr_seq
    }

    /// Attempt to iterate and move our current state forward
    fn iterate<NT>(&mut self, network_node: &Arc<NT>, timeouts: &Timeouts) where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            NetworkNodeState::Init => {
                let known_nodes: Vec<NodeId> = self.network_view.known_nodes().into_iter()
                    .filter(|node| *node != self.network_view.node_id()).collect();

                let join_message = ReconfigurationMessage::NetworkReconfig(NetworkReconfigMessage::new(
                    self.next_seq(),
                    NetworkReconfigMsgType::NetworkJoinRequest(self.network_view.node_triple()),
                ));

                let contacted = known_nodes.len();

                if known_nodes.is_empty() {
                    info!("No known nodes, joining network as a stable member");
                    self.current_state = NetworkNodeState::StableMember;
                }

                let mut node_results = Vec::new();

                for node in &known_nodes {
                    info!("{:?} // Connecting to node {:?}", self.network_view.node_id(), node);
                    let mut node_connection_results = network_node.node_connections().connect_to_node(*node);

                    node_results.push((*node, node_connection_results));
                }

                for (node, conn_results) in node_results {
                    for conn_result in conn_results {
                        if let Err(err) = conn_result.recv().unwrap() {
                            error!("Error while connecting to another node: {:?}", err);
                        }
                    }
                }

                info!("Broadcasting reconfiguration network join message");

                let _ = network_node.broadcast_reconfig_message(join_message, known_nodes.into_iter());

                timeouts.timeout_reconfig_request(TIMEOUT_DUR, (contacted / 2 + 1) as u32, self.curr_seq);

                self.current_state = NetworkNodeState::JoiningNetwork {
                    contacted,
                    responded: Default::default(),
                };
            }
            NetworkNodeState::JoiningNetwork { contacted, responded } => {}
            NetworkNodeState::StableMember => {}
            NetworkNodeState::LeavingNetwork => {}
            NetworkNodeState::IntroductionPhase { .. } => {}
        }
    }

    fn handle_network_reconfig_msg<NT>(&mut self, network_node: &Arc<NT>,timeouts: &Timeouts, header: Header, message: NetworkReconfigMessage)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            NetworkNodeState::JoiningNetwork { contacted, responded } => {
                let (seq, message) = message.into_inner();

                // Avoid accepting double answers

                match message {
                    NetworkReconfigMsgType::NetworkJoinRequest(join_request) => {
                        info!("Received a network join request from {:?} while joining the network", header.from());
                        self.handle_join_request(network_node, header, seq, join_request);
                    }
                    NetworkReconfigMsgType::NetworkJoinResponse(join_response) => {
                        if seq != self.curr_seq {
                            warn!("Received a message with an invalid sequence number while joining the network {:?} vs {:?} (Ours)", seq, self.curr_seq);
                            return;
                        }

                        if responded.insert(header.from()) {
                            match join_response {
                                NetworkJoinResponseMessage::Successful(network_information) => {
                                    info!("We were accepted into the network by the node {:?}", header.from());

                                    timeouts.cancel_reconfig_timeout(Some(self.curr_seq));

                                    self.network_view.handle_successfull_network_join(network_information);

                                    self.current_state = NetworkNodeState::IntroductionPhase {
                                        contacted: 0,
                                        responded: Default::default(),
                                    };
                                }
                                NetworkJoinResponseMessage::Rejected(rejection_reason) => {
                                    error!("We were rejected from the network: {:?} by the node {:?}", rejection_reason, header.from());

                                    timeouts.received_reconfig_request(header.from(), seq);
                                }
                            }
                        }
                    }
                    NetworkReconfigMsgType::NetworkHelloRequest(hello_request) => {
                        info!("Received a network hello request from {:?} but we are still not part of the network", header.from());

                        self.network_view.handle_node_introduced(hello_request);
                    }
                }
            }
            NetworkNodeState::IntroductionPhase { .. } => {}
            NetworkNodeState::StableMember => {

                let (seq, message) = message.into_inner();

                match message {
                    NetworkReconfigMsgType::NetworkJoinRequest(join_request) => {
                        self.handle_join_request(network_node, header, seq, join_request);
                    }
                    NetworkReconfigMsgType::NetworkJoinResponse(_) => {
                        // Ignored, we are already a stable member of the network
                    }
                    NetworkReconfigMsgType::NetworkHelloRequest(hello) => {
                        self.network_view.handle_node_introduced(hello);
                    }
                }
            }
            NetworkNodeState::LeavingNetwork => {}
            _ => {
                //Nothing to do here
            }
        }
    }

    fn handle_join_request<NT>(&self, network_node: &Arc<NT>, header: Header, seq: SeqNo, node: NodeTriple)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let network = network_node.clone();

        let target = header.from();

        let network_view = self.network_view.clone();

        rt::spawn(async move {
            let result = network_view.can_introduce_node(node).await;

            let reconfig_message = NetworkReconfigMessage::new(seq, NetworkReconfigMsgType::NetworkJoinResponse(result));

            let _ = network.send_reconfig_message(ReconfigurationMessage::NetworkReconfig(reconfig_message), target);
        });
    }
}

impl<NT> ReconfigurableNode<NT> where NT: Send + 'static {
    fn run(mut self) where NT: ReconfigurationNode<ReconfData> + 'static {
        loop {

            self.node.iterate(&self.network_node, &self.timeouts);

            self.node_type.iterate(&self.node, &self.network_node);

            let optional_message = self.network_node.reconfiguration_message_handler().try_receive_reconfig_message().unwrap();

            if let Some(message) = optional_message {
                let (header, message) = message.into_inner();

                match message {
                    ReconfigurationMessage::NetworkReconfig(network_reconfig) => {
                        self.node.handle_network_reconfig_msg(&self.network_node, &self.timeouts, header, network_reconfig);
                    }
                    ReconfigurationMessage::QuorumReconfig(quorum_reconfig) => {
                        self.node_type.handle_view_state_message(&self.node, &self.network_node, header, quorum_reconfig);
                    }
                }
            }
        }
    }

    fn handle_local_messages(&mut self) where NT: ReconfigurationNode<ReconfData> + 'static {
        while let Ok(received_message) = self.channel_rx.try_recv() {
            match received_message {
                ReconfigMessage::TimeoutReceived(timeout) => {
                    info!("We have received timeouts {:?}", timeout)
                }
            }
        }
    }
}

impl ReconfigurationProtocol for ReconfigurableNodeProtocol {
    type Config = ReconfigurableNetworkConfig;
    type InformationProvider = NetworkInfo;
    type Serialization = ReconfData;

    fn init_default_information(config: Self::Config) -> Result<Arc<Self::InformationProvider>> {
        Ok(Arc::new(NetworkInfo::init_from_config(config)))
    }

    async fn initialize_protocol<NT>(information: Arc<Self::InformationProvider>,
                                     node: Arc<NT>, timeouts: Timeouts,
                                     node_type: ReconfigurableNodeTypes<QuorumJoinCert<Self::Serialization>>)
                                     -> Result<Self> where NT: ReconfigurationNode<Self::Serialization> + 'static, Self: Sized {
        let general_info = GeneralNodeInfo {
            curr_seq: SeqNo::ZERO,
            current_state: NetworkNodeState::Init,
            network_view: information.clone(),
        };

        let quorum_view = Arc::new(RwLock::new(QuorumView::empty()));

        let node_type = match node_type {
            ReconfigurableNodeTypes::Client => {
                NodeType::Client(ClientQuorumView::new(quorum_view.clone()))
            }
            ReconfigurableNodeTypes::Replica(channel_tx, channel_rx) => {
                NodeType::Replica(ReplicaQuorumView::new(quorum_view.clone(), channel_tx, channel_rx))
            }
        };

        let (channel_tx, channel_rx) = channel::new_bounded_sync(128);

        let reconfigurable_node = ReconfigurableNode {
            node: general_info,
            network_node: node.clone(),
            timeouts,
            channel_rx,
            node_type,
        };

        std::thread::Builder::new()
            .name(format!("Reconfiguration Protocol Thread"))
            .spawn(move || {
                reconfigurable_node.run();
            }).expect("Failed to launch reconfiguration protocol thread");

        let node_handle = ReconfigurableNodeProtocol {
            network_info: information.clone(),
            quorum_info: quorum_view,
            channel_tx,
        };

        Ok(node_handle)
    }

    fn handle_timeout(&self, timeouts: Vec<RqTimeout>) -> Result<ReconfigResponse>{

        self.channel_tx.send(ReconfigMessage::TimeoutReceived(timeouts)).unwrap();

        Ok(ReconfigResponse::Running)
    }

    fn get_quorum_members(&self) -> Vec<NodeId> {
        self.quorum_info.read().unwrap().quorum_members().clone()
    }

    fn is_join_certificate_valid(&self, certificate: &QuorumJoinCert<Self::Serialization>) -> bool {
        todo!()
    }
}
