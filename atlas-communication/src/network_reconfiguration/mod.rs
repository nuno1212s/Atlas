use crate::message::{NetworkMessage, NetworkMessageKind, StoredMessage};
use crate::serialize::Serializable;
use crate::{Node, NodePK, QuorumReconfigurationHandling};
use atlas_common::async_runtime as rt;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx, TryRecvError};
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_reconfiguration::message::{NetworkConfigurationMessage, NetworkJoinResponseMessage};
use atlas_reconfiguration::NetworkNode;
use log::error;
use std::sync::Arc;

pub struct ReconfigurableNetworkNode {
    node_info: Arc<NetworkNode>,

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
}

/// The result of attempting to boostrap the node into the existing network.
/// 
pub enum BoostrapResult {
    Ready,
    Boostrapping,
    JoiningNetwork,
}

impl ReconfigurableNetworkNode {
    pub fn bootstrap_node<NT, M>(&self, node: Arc<NT>) -> Result<BoostrapResult>
    where
        M: Serializable + 'static,
        NT: Node<M> + 'static,
    {

        let bootstrap_nodes : Vec<NodeId> = self.node_info.known_nodes().into_iter().filter(|node_id| {
            if *node_id == self.node_info.node_id() {
                false
            } else {
                true
            }
        }).collect();

        if bootstrap_nodes.len() == 0 {
            // We either know no nodes or we are the only bootstrap node.

            return Ok(BoostrapResult::Ready);
        }

        let our_triple = self.node_info.node_triple();

        let nmk = NetworkConfigurationMessage::NetworkJoinRequest(our_triple);

        node.broadcast_signed(
            NetworkMessageKind::ReconfigurationMessage(nmk),
            bootstrap_nodes.into_iter(),
        );

        Ok(BoostrapResult::JoiningNetwork)
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

                    node.send_signed(
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
                    }
                    NetworkJoinResponseMessage::Rejected(rejected_join) => {
                        error!("Failed to join network because of {:?}", rejected_join);
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
