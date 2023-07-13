extern crate core;

pub mod config;
pub mod message;
pub mod network_reconfig;
pub mod quorum_reconfig;
mod metrics;
mod node_types;

use std::collections::BTreeSet;
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use atlas_common::error::*;
use std::sync::Arc;
use log::{error, info};
use atlas_common::channel;
use atlas_common::async_runtime as rt;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::Header;
use atlas_communication::NodeConnections;
use atlas_communication::reconfiguration_node::{ReconfigurationIncomingHandler, ReconfigurationNode};
use atlas_core::reconfiguration_protocol::{QuorumJoinCert, QuorumReconfigurationMessage, QuorumReconfigurationResponse, ReconfigurableNodeTypes, ReconfigurationProtocol};
use crate::config::ReconfigurableNetworkConfig;
use crate::message::{NetworkJoinResponseMessage, NetworkReconfigMessage, QuorumJoinCertificate, ReconfData, ReconfigurationMessage};
use crate::network_reconfig::NetworkInfo;
use crate::node_types::NodeType;
use crate::quorum_reconfig::QuorumView;

/// The reconfiguration module.
/// Provides various utilities for allowing reconfiguration of the network
/// Such as message definitions, important types and etc.
///
/// This module will then be used by the parts of the system which must be reconfigurable
/// (For example, the network

pub fn bootstrap_reconf_node<NT>(network_info: Arc<NetworkInfo>, node: Arc<NT>)
                                 -> Result<(ChannelSyncRx<QuorumReconfigurationMessage<QuorumJoinCertificate>>,
                                            ChannelSyncTx<QuorumReconfigurationResponse>)>
    where NT: ReconfigurationNode<ReconfData> + 'static {
    let (reconfig_tx, reconfig_rx) = channel::new_bounded_sync(128);

    let (reconfig_resp_tx, reconfig_resp_rx) = channel::new_bounded_sync(128);

    let reconf_node = GeneralNodeInfo {
        curr_seq: SeqNo::ZERO,
        network_view: network_info,
        current_state: NetworkNodeState::Init,
    };

    let reconf_node = ReconfigurableNode {
        node: reconf_node,
        network_node: node,
        node_type: NodeType::new_replica(),
    };

    std::thread::Builder::new()
        .name(format!("Reconfiguration Node Thread"))
        .spawn(move || {
            reconf_node.run();
        }).unwrap();

    Ok((reconfig_rx, reconfig_resp_tx))
}

/// The reconfigurable node which will handle all reconfiguration requests
/// This handles the network level reconfiguration, not the quorum level reconfiguration
pub struct GeneralNodeInfo {
    curr_seq: SeqNo,
    /// Our current view of the network, including nodes we know about, their addresses and public keys
    network_view: Arc<NetworkInfo>,
    /// The current state of the network node, to keep track of which protoclos we are executing
    current_state: NetworkNodeState,
}

/// A reconfigurable node, used to handle the reconfiguration of the network as a whole
pub struct ReconfigurableNode<NT> where NT: Send + 'static {
    /// The general information about the network
    node: GeneralNodeInfo,
    /// The reference to the network node
    network_node: Arc<NT>,
    /// The type of the node we are running.
    node_type: NodeType<NT>,
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
    /// Attempt
    fn iterate<NT>(&mut self, network_node: &Arc<NT>) where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            NetworkNodeState::Init => {
                let known_nodes: Vec<NodeId> = self.network_view.known_nodes().into_iter()
                    .filter(|node| *node != self.network_view.node_id()).collect();

                let join_message = ReconfigurationMessage::NetworkReconfig(NetworkReconfigMessage::NetworkJoinRequest(
                    self.network_view.node_triple()
                ));

                let contacted = known_nodes.len();

                if known_nodes.is_empty() {
                    self.current_state = NetworkNodeState::StableMember;
                }

                let mut node_results = Vec::new();

                for node in &known_nodes {
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

                let _ = network_node.broadcast_reconfig_message(join_message, known_nodes.into_iter());

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

    fn handle_network_reconfig_msg<NT>(&mut self, network_node: Arc<NT>, header: Header, message: NetworkReconfigMessage)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            NetworkNodeState::JoiningNetwork { contacted, responded } => {
                // Avoid accepting double answers
                if responded.insert(header.from()) {
                    match message {
                        NetworkReconfigMessage::NetworkJoinRequest(_) => {
                            info!("Received a network join request from {:?} but we are still not part of the network, so we can't answer", header.from());
                        }
                        NetworkReconfigMessage::NetworkJoinResponse(join_response) => {
                            match join_response {
                                NetworkJoinResponseMessage::Successful(network_information) => {
                                    info!("We were accepted into the network by the node {:?}", header.from());

                                    self.network_view.handle_successfull_network_join(network_information);

                                    self.current_state = NetworkNodeState::IntroductionPhase {
                                        contacted: 0,
                                        responded: Default::default(),
                                    };
                                }
                                NetworkJoinResponseMessage::Rejected(rejection_reason) => {
                                    error!("We were rejected from the network: {:?} by the node {:?}", rejection_reason, header.from());
                                }
                            }
                        }
                        NetworkReconfigMessage::NetworkHelloRequest(hello_request) => {
                            /// Well how do we handle this?
                            info!("Received a network hello request from {:?} but we are still not part of the network, so we can't really process it", header.from());
                        }
                    }
                }
            }
            NetworkNodeState::IntroductionPhase { .. } => {}
            NetworkNodeState::StableMember => {
                match message {
                    NetworkReconfigMessage::NetworkJoinRequest(join_request) => {
                        let network = network_node.clone();

                        let target = header.from();

                        let network_view = self.network_view.clone();

                        rt::spawn(async move {
                            let result = network_view.can_introduce_node(join_request).await;

                            let _ = network.send_reconfig_message(ReconfigurationMessage::NetworkReconfig(NetworkReconfigMessage::NetworkJoinResponse(result)), target);
                        });
                    }
                    NetworkReconfigMessage::NetworkJoinResponse(_) => {
                        // Ignored, we are already a stable member of the network
                    }
                    NetworkReconfigMessage::NetworkHelloRequest(hello) => {
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
}

impl<NT> ReconfigurableNode<NT> where NT: Send + 'static {

    fn run(mut self) where NT: ReconfigurationNode<ReconfData> + 'static {
        loop {
            self.node.iterate(&self.network_node);

            self.node_type.iterate(&self.node, &self.network_node);
        }
    }
}

impl<NT> ReconfigurationProtocol<NT> for ReconfigurableNode<NT> where NT: Send + Sync + 'static {
    type Config = ReconfigurableNetworkConfig;
    type InformationProvider = NetworkInfo;
    type Serialization = ReconfData;

    fn init_default_information(config: Self::Config) -> Result<Arc<Self::InformationProvider>> {
        Ok(Arc::new(NetworkInfo::init_from_config(config)))
    }

    fn initialize_protocol(information: Arc<Self::InformationProvider>, node: Arc<NT>, node_type: ReconfigurableNodeTypes<QuorumJoinCert<Self::Serialization>>) -> Result<Self> where NT: ReconfigurationNode<Self::Serialization> + 'static, Self: Sized {
        todo!()
    }

    fn get_quorum_members(&self) -> Vec<NodeId> {
        todo!()
    }

    fn is_join_certificate_valid(&self, certificate: &QuorumJoinCert<Self::Serialization>) -> bool {
        todo!()
    }
}