#![feature(async_fn_in_trait)]

use std::sync::Arc;
use std::time::Duration;
use crate::serialize::Serializable;
use atlas_common::error::*;
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use atlas_common::channel::OneShotRx;
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::node_id::NodeId;
use crate::reconfiguration_node::{NetworkInformationProvider, ReconfigurationNode};
use crate::protocol_node::ProtocolNetworkNode;

pub mod serialize;
pub mod message;
pub mod cpu_workers;
pub mod client_pooling;
pub mod config;
pub mod message_signing;
pub mod metric;
pub mod reconfiguration_node;
pub mod protocol_node;
pub mod conn_utils;

/// Actual node implementations
//pub mod tcpip;
//pub mod tcp_ip_simplex;
pub mod mio_tcp;

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

/// A full network node implementation
pub trait FullNetworkNode<NI, RM, PM>: ProtocolNetworkNode<PM> + ReconfigurationNode<RM> + Send + Sync
    where
        NI: NetworkInformationProvider,
        RM: Serializable + 'static,
        PM: Serializable + 'static {

    /// The configuration type this node wants to accept
    type Config;

    /// Bootstrap the node
    async fn bootstrap(network_info_provider: Arc<NI>, node_config: Self::Config) -> Result<Self>
        where Self: Sized;
}
