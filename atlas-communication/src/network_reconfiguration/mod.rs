use crate::message::{NetworkMessage, NetworkMessageKind, StoredMessage};
use crate::serialize::Serializable;
use crate::{Node, NodePK, QuorumReconfigurationHandling, NodeConnections};
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx, TryRecvError};
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::peer_addr::PeerAddr;
use atlas_common::{async_runtime as rt, channel};
use atlas_reconfiguration::config::ReconfigurableNetworkConfig;
use atlas_reconfiguration::message::{NetworkConfigurationMessage, NetworkJoinResponseMessage};
use atlas_reconfiguration::NetworkInfo;
use futures::future::join_all;
use log::{debug, error, info};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub enum CurrentNetworkState {
    Init,
    JoiningNetwork(usize, usize),
    Stable,
    LeavingNetwork,
}

pub struct ReconfigurableNetworkNode {
    node_info: Arc<NetworkInfo>,

    current_state: Mutex<CurrentNetworkState>,

    reconfiguration_message_handling: (
        ChannelSyncTx<StoredMessage<NetworkConfigurationMessage>>,
        ChannelSyncRx<StoredMessage<NetworkConfigurationMessage>>,
    ),
}

impl NodePK for ReconfigurableNetworkNode {
    fn get_public_key(&self, node: &NodeId) -> Option<PublicKey> {
        self.node_info.get_pk_for_node(node)
    }

    fn get_key_pair(&self) -> &Arc<KeyPair> {
        self.node_info.keypair()
    }
}

impl QuorumReconfigurationHandling for ReconfigurableNetworkNode {
    fn try_receive_reconfiguration_messages(
        &self,
    ) -> Result<Option<StoredMessage<NetworkConfigurationMessage>>> {
        match self.reconfiguration_message_handling.1.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(e) => match e {
                TryRecvError::Timeout | TryRecvError::ChannelEmpty => Ok(None),
                TryRecvError::ChannelDc => Err(Error::simple_with_msg(
                    ErrorKind::CommunicationChannel,
                    "Channel disconnected",
                )),
            },
        }
    }

    fn receive_reconfiguration_messages(
        &self,
    ) -> Result<StoredMessage<NetworkConfigurationMessage>> {
        match self.reconfiguration_message_handling.1.recv() {
            Ok(msg) => Ok(msg),
            Err(e) => Err(Error::simple_with_msg(
                ErrorKind::CommunicationChannel,
                "Channel disconnected",
            )),
        }
    }

    fn get_peer_address(&self, peer_id: NodeId) -> Option<PeerAddr> {
        self.node_info.get_addr_for_node(&peer_id)
    }
}

/// The result of attempting to boostrap the node into the existing network.
///
pub enum BoostrapResult {
    Ready,
    Boostrapping,
    Failed,
}

impl ReconfigurableNetworkNode {
    pub fn initialize(config: ReconfigurableNetworkConfig) -> Self {
        let channel = channel::new_bounded_sync(100);

        Self {
            node_info: Arc::new(NetworkInfo::init_from_config(config)),
            current_state: Mutex::new(CurrentNetworkState::Init),
            reconfiguration_message_handling: channel,
        }
    }

    pub fn get_own_addr(&self) -> &PeerAddr {
        self.node_info.get_own_addr()
    }

    pub async fn start_bootstrap_process<NT, M>(&self, node: Arc<NT>) -> Result<BoostrapResult>
    where
        M: Serializable + 'static,
        NT: Node<M> + 'static,
    {
        let bootstrap_nodes: Vec<NodeId> = self
            .node_info
            .known_nodes()
            .into_iter()
            .filter(|node_id| {
                if *node_id == self.node_info.node_id() {
                    false
                } else {
                    true
                }
            })
            .collect();

        if bootstrap_nodes.len() == 0 {
            // We either know no nodes or we are the only bootstrap node, in which case we are already ready to start
            // Operations.
            debug!("No bootstrap nodes, ready to start operations. (Or we are the known bootstrap node)");

            let mut current_state = self.current_state.lock().unwrap();

            *current_state = CurrentNetworkState::Stable;

            return Ok(BoostrapResult::Ready);
        }

        {
            let mut current_state = self.current_state.lock().unwrap();

            *current_state = CurrentNetworkState::JoiningNetwork(bootstrap_nodes.len(), 0);
        }

        let our_triple = self.node_info.node_triple();

        let nmk = NetworkConfigurationMessage::NetworkJoinRequest(our_triple);

        let mut result_vec = Vec::new();

        for node_id in &bootstrap_nodes {
            debug!("Connecting to bootstrap node: {:?}", node_id);

            let conn_results = node.node_connections().connect_to_node(node_id.clone());

            let result = join_all(conn_results);

            result_vec.push(result);
        }

        let results = join_all(result_vec).await;

        for node_conn_res in results {

            for conn_result in node_conn_res {
                let conn_res = conn_result.expect("Failed to receive from conn attempt?");

                if let Err(err) = conn_res {
                    error!("Failed to connect to node: {:?}", err);

                    break
                }
            }
        }

        debug!(
            "Broadcasting network join request to bootstrap nodes: {:?}",
            bootstrap_nodes
        );

        // Ignoring the results atm
        let _ = node.broadcast_signed(
            NetworkMessageKind::ReconfigurationMessage(nmk),
            bootstrap_nodes.into_iter(),
        );

        Ok(BoostrapResult::Boostrapping)
    }

    pub fn handle_message_received<M, NT>(
        self: &Arc<Self>,
        msg: NetworkMessage<M>,
        node: &Arc<NT>,
    ) -> Result<BoostrapResult>
    where
        M: Serializable + 'static,
        NT: Node<M> + 'static,
    {
        let (header, message) = msg.into_inner();

        let reconfiguration_message = match message {
            NetworkMessageKind::ReconfigurationMessage(reconfig) => reconfig,
            _ => unreachable!(),
        };

        match reconfiguration_message {
            NetworkConfigurationMessage::NetworkJoinRequest(node_triple) => {
                let network_node = Arc::clone(self);
                let node = Arc::clone(node);

                rt::spawn(async move {
                    let target = node_triple.node_id();

                    let result = Arc::clone(&network_node.node_info)
                        .can_introduce_node(node_triple)
                        .await;

                    let njr = NetworkConfigurationMessage::NetworkJoinResponse(result);

                    let _ = node.send_signed(
                        NetworkMessageKind::ReconfigurationMessage(njr),
                        target,
                        true,
                    );
                });
            }
            NetworkConfigurationMessage::NetworkJoinResponse(join_response) => {
                match join_response {
                    NetworkJoinResponseMessage::Successful(network_view) => {
                        self.node_info.handle_successfull_network_join(network_view);

                        let mut state_guard = self.current_state.lock().unwrap();

                        //TODO: Should this first match to see what is the current state and if we are already part of the network then we ignore it?

                        *state_guard = CurrentNetworkState::Stable;

                        return Ok(BoostrapResult::Ready);
                    }
                    NetworkJoinResponseMessage::Rejected(rejected_join) => {
                        let mut state_guard = self.current_state.lock().unwrap();

                        error!("Failed to join network because of {:?}", rejected_join);

                        match &mut *state_guard {
                            CurrentNetworkState::Init => {}
                            CurrentNetworkState::JoiningNetwork(
                                sent_requests,
                                received_responses,
                            ) => {
                                *received_responses += 1;

                                if received_responses >= sent_requests {
                                    return Ok(BoostrapResult::Failed);
                                }
                            }
                            CurrentNetworkState::Stable => {
                                info!("Received a negative network join response while already in the network. Ignoring.");
                            }
                            CurrentNetworkState::LeavingNetwork => {
                                info!("Received a negative network join response while already leaving the network. Ignoring.");
                            }
                        }
                    }
                }
            }
            _ => {
                let message = StoredMessage::new(header, reconfiguration_message);

                if let Err(err) = self.reconfiguration_message_handling.0.send(message) {
                    error!("Failed to send reconfiguration message to the message channel.");
                }
            }
        }

        Ok(BoostrapResult::Boostrapping)
    }
}