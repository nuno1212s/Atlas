use crate::message::{StoredMessage};
use crate::serialize::Serializable;
use crate::{NodeConnections};
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx, TryRecvError};
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::error::*;
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::peer_addr::PeerAddr;
use atlas_common::channel;

use std::sync::{Arc};
use std::time::Duration;

/// Represents the network information that a node needs to know about other nodes
pub trait NetworkInformationProvider: Send + Sync {
    /// Get the node id of our own node
    fn get_own_addr(&self) -> PeerAddr;

    /// Get our own key pair
    fn get_key_pair(&self) -> &Arc<KeyPair>;

    /// Get your own node type
    fn get_own_node_type(&self) -> NodeType;

    /// Get the node type for a given node
    fn get_node_type(&self, node: &NodeId) -> Option<NodeType>;

    /// Get the public key of a given node
    fn get_public_key(&self, node: &NodeId) -> Option<PublicKey>;

    /// Get the peer addr for a given node
    fn get_addr_for_node(&self, node: &NodeId) -> Option<PeerAddr>;
}

/// Handling of incoming requests
pub trait ReconfigurationIncomingHandler<T> {
    /// Receive a reconfiguration message from other nodes
    fn receive_reconfig_message(&self) -> Result<T>;

    /// Try to receive a reconfiguration message from other nodes
    /// If no messages are already available at the time of the call, then it will return None
    fn try_receive_reconfig_message(&self, timeout: Option<Duration>) -> Result<Option<T>>;
}

/// The reconfiguration network update trait, to send updates about newly discovered
/// nodes to the networking layer. This is because when a new node connects to us and
/// we don't know about it, it will be left in a pending state until we receive a
/// network update telling us about this new node. In this pending state we will
/// only receive reconfiguration messages from this node.
pub trait ReconfigurationNetworkUpdate {
    fn send_reconfiguration_update(&self, update: NetworkUpdateMessage) -> Result<()>;
}

/// Trait for handling reconfiguration messages and etc
pub trait ReconfigurationNode<M>: Send + Sync where M: Serializable + 'static {
    type ConnectionManager: NodeConnections;

    type NetworkInfoProvider: NetworkInformationProvider;

    type IncomingReconfigRqHandler: ReconfigurationIncomingHandler<StoredMessage<M::Message>>;

    type ReconfigurationNetworkUpdate: ReconfigurationNetworkUpdate;

    /// The connection manager for this node
    fn node_connections(&self) -> &Arc<Self::ConnectionManager>;

    /// The network information provider for this node
    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider>;

    /// The network update handler for the reconfiguration protocol to deliver updates to
    /// the networking layer
    fn reconfiguration_network_update(&self) -> &Arc<Self::ReconfigurationNetworkUpdate>;

    /// Get the handler to the incoming reconfiguration messages
    fn reconfiguration_message_handler(&self) -> &Arc<Self::IncomingReconfigRqHandler>;

    /// Send a reconfiguration message to a given target node
    fn send_reconfig_message(&self, message: M::Message, target: NodeId) -> Result<()>;

    /// Broadcast a reconfiguration message to a given set of nodes.
    fn broadcast_reconfig_message(&self, message: M::Message, target: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>>;
}

#[derive(Clone)]
pub enum NetworkUpdateMessage {
    NodeConnectionPermitted(NodeId, NodeType, PublicKey)
}

pub struct ReconfigurationMessageHandler<T> {
    reconfiguration_message_handling: (
        ChannelSyncTx<T>,
        ChannelSyncRx<T>,
    ),
    update_message_handling: (
        ChannelSyncTx<NetworkUpdateMessage>,
        ChannelSyncRx<NetworkUpdateMessage>,
    ),
}

impl<T> ReconfigurationMessageHandler<T> {
    pub fn initialize() -> Self {
        ReconfigurationMessageHandler {
            reconfiguration_message_handling: channel::new_bounded_sync(100),
            update_message_handling: channel::new_bounded_sync(100),
        }
    }

    pub fn push_request(&self, message: T) -> Result<()> {
        if let Err(err) = self.reconfiguration_message_handling.0.send(message) {
            return Err(Error::simple_with_msg(ErrorKind::CommunicationChannel, format!("Failed to send reconfiguration message to the message channel. Error: {:?}", err).as_str()));
        }

        return Ok(());
    }
}

impl<T> ReconfigurationMessageHandler<T> {
    pub fn receive_network_update(&self) -> Result<NetworkUpdateMessage> {
        Ok(self.update_message_handling.1.recv().unwrap())
    }

    pub fn try_receive_network_update(&self, timeout: Option<Duration>) -> Result<Option<NetworkUpdateMessage>> {
        if let Some(timeout) = timeout {
            match self.update_message_handling.1.recv_timeout(timeout) {
                Ok(msg) => {
                    Ok(Some(msg))
                }
                Err(err) => {
                    match err {
                        TryRecvError::ChannelEmpty | TryRecvError::Timeout => {
                            Ok(None)
                        }
                        TryRecvError::ChannelDc => {
                            Err(Error::simple_with_msg(ErrorKind::CommunicationChannel, "Reconfig message channel has disconnected?"))
                        }
                    }
                }
            }
        } else {
            match self.update_message_handling.1.try_recv() {
                Ok(msg) => {
                    Ok(Some(msg))
                }
                Err(err) => {
                    match err {
                        TryRecvError::ChannelEmpty | TryRecvError::Timeout => {
                            Ok(None)
                        }
                        TryRecvError::ChannelDc => {
                            Err(Error::simple_with_msg(ErrorKind::CommunicationChannel, "Reconfig message channel has disconnected?"))
                        }
                    }
                }
            }
        }
    }
}

impl<T> ReconfigurationIncomingHandler<T> for ReconfigurationMessageHandler<T> {
    fn receive_reconfig_message(&self) -> Result<T> {
        self.reconfiguration_message_handling.1.recv().wrapped_msg(ErrorKind::CommunicationChannel, "Failed to receive message")
    }

    fn try_receive_reconfig_message(&self, timeout: Option<Duration>) -> Result<Option<T>> {
        if let Some(timeout) = timeout
        {
            match self.reconfiguration_message_handling.1.recv_timeout(timeout) {
                Ok(msg) => {
                    Ok(Some(msg))
                }
                Err(err) => {
                    match err {
                        TryRecvError::ChannelEmpty | TryRecvError::Timeout => {
                            Ok(None)
                        }
                        TryRecvError::ChannelDc => {
                            Err(Error::simple_with_msg(ErrorKind::CommunicationChannel, "Reconfig message channel has disconnected?"))
                        }
                    }
                }
            }
        } else {
            match self.reconfiguration_message_handling.1.try_recv() {
                Ok(msg) => {
                    Ok(Some(msg))
                }
                Err(err) => {
                    match err {
                        TryRecvError::ChannelEmpty | TryRecvError::Timeout => {
                            Ok(None)
                        }
                        TryRecvError::ChannelDc => {
                            Err(Error::simple_with_msg(ErrorKind::CommunicationChannel, "Reconfig message channel has disconnected?"))
                        }
                    }
                }
            }
        }
    }
}

impl<T> ReconfigurationNetworkUpdate for ReconfigurationMessageHandler<T> {
    fn send_reconfiguration_update(&self, update: NetworkUpdateMessage) -> Result<()> {
        self.update_message_handling.0.send(update);

        Ok(())
    }
}
