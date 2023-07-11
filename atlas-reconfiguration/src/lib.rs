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
use crate::message::{NetworkJoinResponseMessage, NetworkReconfigMessage, NodeTriple, QuorumReconfigMessage, QuorumReconfigurationMessage, QuorumReconfigurationResponse, ReconfData, ReconfigurationMessage};
use crate::network_reconfig::NetworkInfo;
use crate::node_types::NodeType;
use crate::quorum_reconfig::{QuorumView, QuorumNode};

/// The reconfiguration module.
/// Provides various utilities for allowing reconfiguration of the network
/// Such as message definitions, important types and etc.
///
/// This module will then be used by the parts of the system which must be reconfigurable
/// (For example, the network

pub fn bootstrap_reconf_node<NT>(network_info: Arc<NetworkInfo>, node: Arc<NT>)
                                 -> Result<(ChannelSyncRx<QuorumReconfigurationMessage>,
                                            ChannelSyncTx<QuorumReconfigurationResponse>)>
    where NT: ReconfigurationNode<ReconfData> + 'static {
    let (reconfig_tx, reconfig_rx) = channel::new_bounded_sync(128);

    let (reconfig_resp_tx, reconfig_resp_rx) = channel::new_bounded_sync(128);

    let reconf_node = ReconfigurableNode {
        curr_seq: SeqNo::ZERO,
        network_view: network_info,
        current_state: NetworkNodeState::Init,
        quorum_communication: reconfig_tx,
        quorum_responses: reconfig_resp_rx,
        network_node: node,
        node_type: NodeType::Client(),
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
struct ReconfigurableNode<NT> {
    curr_seq: SeqNo,

    network_view: Arc<NetworkInfo>,

    current_state: NetworkNodeState,

    quorum_communication: ChannelSyncTx<QuorumReconfigurationMessage>,
    quorum_responses: ChannelSyncRx<QuorumReconfigurationResponse>,

    network_node: Arc<NT>,

    node_type: NodeType,
}


/// The current state of our node (network level, not quorum level)
/// Quorum level operations will only take place when we are a stable member of the protocol
#[derive(Debug, Clone)]
enum NetworkNodeState {
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


/// A node which wants to partake in the consensus protocol
struct ReplicaQuorumNode {
    quorum_view: QuorumNode,

    quorum_node_state: QuorumNodeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuorumNodeState {
    /// We are currently initializing and will attempt to join the network
    Init,
    /// We are currently joining the network and will attempt to acquire the quorum
    JoiningNetwork,
    /// Stable member of the quorum, up to date with the members
    StableMember,
    /// Leaving the quorum
    LeavingNetwork,
}

impl<NT> ReconfigurableNode<NT> {
    fn run(mut self) where NT: ReconfigurationNode<ReconfData> {
        loop {
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

                        continue;
                    }

                    let mut node_results = Vec::new();

                    for node in &known_nodes {
                        let mut node_connection_results = self.network_node.node_connections().connect_to_node(*node);

                        node_results.push((*node, node_connection_results));
                    }

                    for (node, conn_results) in node_results {
                        for conn_result in conn_results {
                            if let Err(err) = conn_result.recv().unwrap() {
                                error!("Error while connecting to another node: {:?}", err);
                            }
                        }
                    }

                    let _ = self.network_node.broadcast_reconfig_message(join_message, known_nodes.into_iter());

                    self.current_state = NetworkNodeState::JoiningNetwork {
                        contacted,
                        responded: Default::default(),
                    };
                }
                NetworkNodeState::JoiningNetwork { contacted, responded } => {
                    let reconfiguration_message = self.network_node.reconfiguration_message_handler().receive_reconfig_message().unwrap();

                    let (header, message): (Header, ReconfigurationMessage) = reconfiguration_message.into_inner();

                    // Avoid accepting double answers
                    if responded.insert(header.from()) {
                        match message {
                            ReconfigurationMessage::NetworkReconfig(network_reconfig_msg) => {
                                match network_reconfig_msg {
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
                                    NetworkReconfigMessage::NetworkViewStateRequest => {}
                                    NetworkReconfigMessage::NetworkViewState(_) => {}
                                }
                            }
                            ReconfigurationMessage::QuorumReconfig(quorum_reconfig) => {
                                // We cannot partake in quorum reconfiguration debacles until we are a stable member of the network
                            }
                        }
                    }
                }
                NetworkNodeState::StableMember => {
                    let reconfiguration_message = self.network_node.reconfiguration_message_handler().receive_reconfig_message().unwrap();

                    let (header, message): (Header, ReconfigurationMessage) = reconfiguration_message.into_inner();

                    match message {
                        ReconfigurationMessage::NetworkReconfig(network_reconfig) => {
                            match network_reconfig {
                                NetworkReconfigMessage::NetworkJoinRequest(join_request) => {
                                    let network = self.network_node.clone();

                                    let target = header.from();

                                    rt::spawn(async move {
                                        let result = self.network_view.can_introduce_node(join_request).await;

                                        let _ = network.send_reconfig_message(ReconfigurationMessage::NetworkReconfig(NetworkReconfigMessage::NetworkJoinResponse(result)), target);
                                    });
                                }
                                NetworkReconfigMessage::NetworkJoinResponse(_) => {
                                    // Ignored, we are already a stable member of the network
                                }
                                NetworkReconfigMessage::NetworkHelloRequest(_) => {}
                            }
                        }
                        ReconfigurationMessage::QuorumReconfig(quorum_reconfig) => {
                            match quorum_reconfig {
                                QuorumReconfigMessage::NetworkViewStateRequest => {
                                    let current_view = self.quorum_view.network_view();

                                    let message = QuorumReconfigMessage::NetworkViewState(current_view);

                                    let _ = self.network_node.send_reconfig_message(ReconfigurationMessage::QuorumReconfig(message), header.from());
                                }
                                QuorumReconfigMessage::NetworkViewState(state_view) => {}
                                QuorumReconfigMessage::QuorumEnterRequest(_) => {}
                                QuorumReconfigMessage::QuorumEnterResponse(_) => {}
                                QuorumReconfigMessage::QuorumLeaveRequest(_) => {}
                                QuorumReconfigMessage::QuorumLeaveResponse(_) => {}
                            }
                        }
                    }
                }
                NetworkNodeState::LeavingNetwork => {}
                NetworkNodeState::IntroductionPhase { .. } => {}
            }
        }
    }

    fn introduce_node(&self) where NT: ReconfigurationNode<ReconfData> {
        let know_nodes = self.network_view.known_nodes();

        let hello_message = NetworkReconfigMessage::NetworkHelloRequest(self.network_view.node_triple());

        let _ = self.network_node.broadcast_reconfig_message(ReconfigurationMessage::NetworkReconfig(hello_message), know_nodes.into_iter());
    }
}