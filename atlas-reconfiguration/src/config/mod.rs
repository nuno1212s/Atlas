use atlas_common::{node_id::NodeId, crypto::signature::KeyPair, peer_addr::PeerAddr};

use crate::message::NodeTriple;


/// The configuration for the reconfiguration network.
pub struct ReconfigurableNetworkConfig {
    // The node ID of this node
    pub node_id: NodeId,
    // The key pair of this node
    pub key_pair: KeyPair,
    // Our address
    pub our_address: PeerAddr,
    // The nodes that we already know about (Boostrap nodes of the network)
    pub known_nodes: Vec<NodeTriple>,
}