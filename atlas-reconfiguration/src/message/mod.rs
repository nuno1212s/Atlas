use std::fmt::Debug;

use atlas_common::crypto::signature::{PublicKey, Signature};
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;

use atlas_common::peer_addr::PeerAddr;
#[cfg(feature = "serialize_serde")]
use serde::{Serialize, Deserialize};

use crate::{KnownNodes, NetworkView};


/// Used to request to join the current quorum
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumEnterRequest {
    node_triple: NodeTriple,
}

/// When a node makes request to join a given network view, the participating nodes
/// must respond with a QuorumNodeJoinResponse.
/// TODO: Decide how many responses we need in order to consider it a valid join request
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumNodeJoinResponse {
    network_view_seq: SeqNo,

    requesting_node: NodeId,
    origin_node: NodeId,
}

/// A certificate composed of enough QuorumNodeJoinResponses to consider that enough
/// existing quorum nodes have accepted the new node
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumJoinCertificate {
    network_view_seq: SeqNo,
    approvals: Vec<QuorumNodeJoinResponse>,
}

/// Reason message for the rejection of quorum entering request
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumEnterRejectionReason {
    NotAuthorized,
    MissingValues,
    IncorrectNetworkViewSeq,
    NodeIsNotQuorumParticipant,
}

/// A response to a network join request
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumEnterResponse {
    Successful(QuorumNodeJoinResponse),

    Rejected(QuorumEnterRejectionReason),
}

#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumLeaveRequest {
    node_triple: NodeTriple,
}

#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumLeaveResponse {
    network_view_seq: SeqNo,

    requesting_node: NodeId,
    origin_node: NodeId,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct KnownNodesMessage {
    nodes: Vec<NodeTriple>,
}

#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct NodeTriple {
    node_id: NodeId,
    addr: PeerAddr,
    pub_key: Vec<u8>
}

/// The response to the request to join the network
/// Returns the list of known nodes in the network, including the newly added node
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum NetworkJoinResponseMessage {
    Successful(KnownNodesMessage),
    Rejected(NetworkJoinRejectionReason),
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum NetworkJoinRejectionReason {
    NotAuthorized,
    MissingValues,
    IncorrectSignature,
    // Clients don't need to connect to other clients, for example, so it is not necessary
    // For them to know about each other
    NotNecessary
}

#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum NetworkConfigurationMessage {
    NetworkJoinRequest(NodeTriple),
    NetworkJoinResponse(NetworkJoinResponseMessage),
    NetworkViewStateRequest,
    NetworkViewState(NetworkView),
    QuorumEnterRequest(QuorumEnterRequest),
    QuorumEnterResponse(QuorumEnterResponse),
    QuorumLeaveRequest(QuorumLeaveRequest),
    QuorumLeaveResponse(QuorumLeaveResponse),
}

impl QuorumNodeJoinResponse {
    pub fn new(network_view_seq: SeqNo, requesting_node: NodeId, origin_node: NodeId) -> Self {
        Self { network_view_seq, requesting_node, origin_node }
    }
}

impl NodeTriple {
    pub fn new(node_id: NodeId, public_key: Vec<u8>, address: PeerAddr) -> Self {
        Self {
            node_id,
            addr: address,
            pub_key: public_key,
        }
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn public_key(&self) -> &Vec<u8> {
        &self.pub_key
    }

    pub fn addr(&self) -> &PeerAddr {
        &self.addr
    }
}

impl From<&KnownNodes> for KnownNodesMessage {
    fn from(value: &KnownNodes) -> Self {

        let mut known_nodes = Vec::with_capacity(value.node_keys.len());

        for node_id in value.node_keys().keys() {
            known_nodes.push(NodeTriple {
                node_id: *node_id,
                pub_key: value.node_key_bytes.get(node_id).unwrap().clone(),
                addr: value.node_addrs.get(node_id).unwrap().clone(),
            });
        }

        KnownNodesMessage {
            nodes: known_nodes,
        }
    }
}

impl KnownNodesMessage {

    pub fn known_nodes(&self) -> &Vec<NodeTriple> {
        &self.nodes
    }

    pub fn into_nodes(self) -> Vec<NodeTriple> {
        self.nodes
    }

}

impl Debug for NodeTriple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeTriple {{ node_id: {:?}, addr: {:?}}}", self.node_id, self.addr)
    }
}