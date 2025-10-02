use atlas_common::node_id::NodeType;
use atlas_common::{crypto::signature::KeyPair, node_id::NodeId, peer_addr::PeerAddr};
use atlas_communication::reconfiguration::NodeInfo;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

/// The configuration for the reconfiguration network.
#[derive()]
pub struct ReconfigurableNetworkConfig {
    // The node ID of this node
    pub node_id: NodeId,
    // The type of this node.
    pub node_type: NodeType,
    // The key pair of this node
    pub key_pair: Arc<KeyPair>,
    // Our address
    pub our_address: PeerAddr,
    // The nodes that we already know about (Boostrap nodes of the network)
    pub known_nodes: Vec<NodeInfo>,
}

impl Debug for ReconfigurableNetworkConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReconfigurableNetworkConfig")
            .field("node_id", &self.node_id)
            .field("node_type", &self.node_type)
            .field("our_address", &self.our_address)
            .field("known_nodes", &self.known_nodes)
            .finish()
    }
}

impl Clone for ReconfigurableNetworkConfig {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id,
            node_type: self.node_type,
            key_pair: self.key_pair.clone(),
            our_address: self.our_address.clone(),
            known_nodes: self.known_nodes.clone(),
        }
    }
}
