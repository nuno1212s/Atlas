#![feature(async_fn_in_trait)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use intmap::IntMap;
use rustls::{ClientConfig, ServerConfig};
use crate::message::{NetworkMessage, NetworkMessageKind, StoredMessage, StoredSerializedNetworkMessage};
use crate::serialize::Serializable;
use atlas_common::error::*;
use crate::client_pooling::{ConnectedPeer, PeerIncomingRqHandling};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use atlas_common::channel::OneShotRx;
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::node_id::NodeId;
use atlas_reconfiguration::message::NetworkConfigurationMessage;
use crate::config::NodeConfig;
use crate::tcpip::{ConnectionType, NodeConnectionAcceptor, TlsNodeAcceptor, TlsNodeConnector};

pub mod serialize;
pub mod message;
pub mod cpu_workers;
pub mod client_pooling;
pub mod config;
pub mod message_signing;
pub mod metric;
pub mod tcpip;
pub mod tcp_ip_simplex;
pub mod mio_tcp;
pub mod network_reconfiguration;

/// A trait defined that indicates how the connections are managed
/// Allows us to verify various things about our current connections as well
/// as establishing new ones.
pub trait NodeConnections {

    /// Are we currently connected to a given node?
    fn is_connected_to_node(&self, node: &NodeId) -> bool;

    /// How many nodes are we currently connected to in this node
    fn connected_nodes_count(&self) -> usize;

    /// Get the nodes we are connected to at this time
    fn connected_nodes(&self) -> Vec<NodeId>;

    /// Connect this node to another node.
    /// This will attempt to create various connections,
    /// depending on the configuration for concurrent connection count.
    /// Returns a vec with the results of each of the attempted connections
    fn connect_to_node(self: &Arc<Self>, node: NodeId) -> Vec<OneShotRx<Result<()>>>;

    /// Disconnect this node from another node
    async fn disconnect_from_node(&self, node: &NodeId) -> Result<()>;

}

pub trait NodePK {

    /// Get the public key for a given node
    fn get_public_key(&self, node: &NodeId) -> Option<PublicKey>;

    /// Get our own key pair
    fn get_key_pair(&self) -> &Arc<KeyPair>;

}

/// Trait for taking requests from the network node
pub trait NodeIncomingRqHandler<T>: Send {

    fn rqs_len_from_clients(&self) -> usize;

    fn receive_from_clients(&self, timeout: Option<Duration>) -> Result<Vec<T>>;

    fn try_receive_from_clients(&self) -> Result<Option<Vec<T>>>;

    fn rqs_len_from_replicas(&self) -> usize;

    fn receive_from_replicas(&self, timeout: Option<Duration>) -> Result<Option<T>>;

}

/// The trait for the handling of reconfiguration messages (Meant for the quorum only)
/// Reconfiguration messages related to the network layer are handled by the network layer
pub trait QuorumReconfigurationHandling {

    /// Attempt to take reconfiguration messages that have been received so they can be processed
    /// by the correct handler. Returns None if no message is available and does not block
    fn try_receive_reconfiguration_messages(&self) -> Result<Option<StoredMessage<NetworkConfigurationMessage>>>;

    /// Take reconfiguration messages that have been received so they can be processed
    /// by the correct handler. Blocks until a message is available
    fn receive_reconfiguration_messages(&self) -> Result<StoredMessage<NetworkConfigurationMessage>>;

}

/// A network node. Handles all the connections between nodes.
pub trait Node<M: Serializable + 'static> : Send + Sync {

    type Config;

    type ConnectionManager : NodeConnections;

    type Crypto: NodePK;

    type IncomingRqHandler: NodeIncomingRqHandler<NetworkMessage<M>>;

    type ReconfigurationHandling: QuorumReconfigurationHandling;

    /// Bootstrap the node
    async fn bootstrap(node_config: Self::Config) -> Result<Arc<Self>>;

    /// Reports the id of this `Node`.
    fn id(&self) -> NodeId;

    /// Reports the first Id
    fn first_cli(&self) -> NodeId;

    /// Get a handle to the connection manager of this node.
    fn node_connections(&self) -> &Arc<Self::ConnectionManager>;

    /// Crypto
    fn pk_crypto(&self) -> &Self::Crypto;

    /// Get a reference to the incoming request handling
    fn node_incoming_rq_handling(&self) -> &Arc<Self::IncomingRqHandler>;

    /// Quorum reconfiguration message request handling.
    fn quorum_reconfig_handling(&self) -> &Arc<Self::ReconfigurationHandling>;

    /// Sends a message to a given target.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send(&self, message: NetworkMessageKind<M>, target: NodeId, flush: bool) -> Result<()>;

    /// Sends a signed message to a given target
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send_signed(&self, message: NetworkMessageKind<M>, target: NodeId, flush: bool) -> Result<()>;

    /// Broadcast a message to all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast(&self, message: NetworkMessageKind<M>, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>>;

    /// Broadcast a signed message for all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_signed(&self, message: NetworkMessageKind<M>, target: impl Iterator<Item = NodeId>) -> std::result::Result<(), Vec<NodeId>>;

    /// Broadcast the serialized messages provided.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_serialized(&self, messages: BTreeMap<NodeId, StoredSerializedNetworkMessage<M>>) -> std::result::Result<(), Vec<NodeId>>;

}