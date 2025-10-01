use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use getset::{CopyGetters, Getters};

use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::channel::TryRecvError;
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::error::*;
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::peer_addr::PeerAddr;
use atlas_common::Err;

#[derive(Clone, Getters, CopyGetters, Debug)]
#[cfg_attr(
    feature = "serialize_serde",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct NodeInfo {
    #[get_copy = "pub"]
    node_id: NodeId,
    #[get_copy = "pub"]
    node_type: NodeType,
    #[get = "pub"]
    public_key: PublicKey,
    #[get = "pub"]
    addr: PeerAddr,
}

/// Represents the network information that a node needs to know about other nodes
pub trait NetworkInformationProvider: Send + Sync {
    /// Get our own node info
    fn own_node_info(&self) -> &NodeInfo;

    /// Get our own key pair
    fn get_key_pair(&self) -> &Arc<KeyPair>;

    /// Get the node info for a given node
    fn get_node_info(&self, node: &NodeId) -> Option<NodeInfo>;
}

/// The reconfiguration network update trait, to send updates about newly discovered
/// nodes to the networking layer. This is because when a new node connects to us and
/// we don't know about it, it will be left in a pending state until we receive a
/// network update telling us about this new node. In this pending state we will
/// only receive reconfiguration messages from this node.
pub trait ReconfigurationNetworkUpdate {
    fn send_reconfiguration_update(
        &self,
        update: ReconfigurationNetworkUpdateMessage,
    ) -> Result<()>;
}

#[derive(Clone)]
pub enum ReconfigurationNetworkUpdateMessage {
    NodeConnectionPermitted(NodeId, NodeType, PublicKey),
}

#[derive(Clone)]
pub enum NetworkUpdatedMessage {
    // We have lost all connection to a given node
    NodeDisconnected(NodeId),
    // The connection to a given node is faulty (e.g. One of the connections failed, etc)
    ConnectionFaulty(NodeId),
}

/// The communication handle for communication between
/// the reconfiguration protocol and the network protocols.
///
/// This handle is made for the Reconfiguration protocol side
#[derive(Clone, Getters)]
pub struct ReconfigurationNetworkCommunication {
    network_update_sender: ChannelSyncTx<ReconfigurationNetworkUpdateMessage>,
    #[get = "pub"]
    network_update_receiver: ChannelSyncRx<NetworkUpdatedMessage>,
}

/// The communication handle for communication between
/// the network protocols and the reconfiguration protocol.
///
/// This handle is made for the network protocol side
#[derive(Clone)]
pub struct NetworkReconfigurationCommunication {
    network_update_receiver: ChannelSyncRx<ReconfigurationNetworkUpdateMessage>,

    network_update_sender: ChannelSyncTx<NetworkUpdatedMessage>,
}

impl ReconfigurationNetworkUpdate for ReconfigurationNetworkCommunication {
    fn send_reconfiguration_update(
        &self,
        update: ReconfigurationNetworkUpdateMessage,
    ) -> Result<()> {
        self.network_update_sender
            .send(update)
            .context("Failed to send reconfiguration message to network update sender channel")
    }
}

impl ReconfigurationNetworkCommunication {
    pub fn receive_network_update(&self) -> Result<NetworkUpdatedMessage> {
        self.network_update_receiver
            .recv()
            .context("Failed to receive reconfiguration message from network update receiver")
    }

    pub fn try_receive_network_update(
        &self,
        timeout: Option<Duration>,
    ) -> Result<Option<NetworkUpdatedMessage>> {
        match timeout {
            None => match self.network_update_receiver.try_recv() {
                Ok(msg) => Ok(Some(msg)),
                Err(err) => match err {
                    TryRecvError::ChannelEmpty | TryRecvError::Timeout => Ok(None),
                    TryRecvError::ChannelDc => {
                        Err!(err)
                    }
                },
            },
            Some(timeout) => match self.network_update_receiver.recv_timeout(timeout) {
                Ok(msg) => Ok(Some(msg)),
                Err(err) => match err {
                    TryRecvError::ChannelEmpty | TryRecvError::Timeout => Ok(None),
                    TryRecvError::ChannelDc => {
                        Err!(err)
                    }
                },
            },
        }
    }
}

impl NetworkReconfigurationCommunication {
    /// Receive a network update from the reconfiguration protocol
    ///
    /// This method will bro block until a network update is received
    pub fn receive_network_update(&self) -> Result<ReconfigurationNetworkUpdateMessage> {
        self.network_update_receiver
            .recv()
            .map_err(|err| err.into())
    }

    /// Try to receive a network update from the reconfiguration protocol
    ///
    /// This method will return immediately with an `Ok(None)` if no network update is available
    pub fn try_receive_network_update(
        &self,
        timeout: Option<Duration>,
    ) -> Result<Option<ReconfigurationNetworkUpdateMessage>> {
        if let Some(timeout) = timeout {
            match self.network_update_receiver.recv_timeout(timeout) {
                Ok(msg) => Ok(Some(msg)),
                Err(err) => match err {
                    TryRecvError::ChannelEmpty | TryRecvError::Timeout => Ok(None),
                    TryRecvError::ChannelDc => {
                        Err(anyhow!("Reconfig message channel has disconnected?"))
                    }
                },
            }
        } else {
            match self.network_update_receiver.try_recv() {
                Ok(msg) => Ok(Some(msg)),
                Err(err) => match err {
                    TryRecvError::ChannelEmpty | TryRecvError::Timeout => Ok(None),
                    TryRecvError::ChannelDc => {
                        Err(anyhow!("Reconfig message channel has disconnected?"))
                    }
                },
            }
        }
    }
}

/// Initialize the network reconfiguration communication channels
///
/// This function will return a tuple of the network reconfiguration communication handles
pub fn initialize_network_reconfiguration_comms(
    channel_capacity: usize,
) -> (
    NetworkReconfigurationCommunication,
    ReconfigurationNetworkCommunication,
) {
    let (network_tx, network_rx) =
        channel::sync::new_bounded_sync(channel_capacity, Some("Network reconfig message channel"));
    let (reconfig_tx, reconfig_rx) =
        channel::sync::new_bounded_sync(channel_capacity, Some("Reconfig network message channel"));

    (
        NetworkReconfigurationCommunication {
            network_update_receiver: network_rx,
            network_update_sender: reconfig_tx,
        },
        ReconfigurationNetworkCommunication {
            network_update_receiver: reconfig_rx,
            network_update_sender: network_tx,
        },
    )
}

impl NodeInfo {
    pub fn new(
        node_id: NodeId,
        node_type: NodeType,
        public_key: PublicKey,
        addr: PeerAddr,
    ) -> Self {
        NodeInfo {
            node_id,
            node_type,
            public_key,
            addr,
        }
    }
}
