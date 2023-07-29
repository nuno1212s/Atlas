use std::fmt::{Debug, Formatter};

#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use atlas_common::crypto::signature::Signature;
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::peer_addr::PeerAddr;
use atlas_communication::message::StoredMessage;
use atlas_communication::serialize::Serializable;
use atlas_core::serialize::ReconfigurationProtocolMessage;
use atlas_core::timeouts::RqTimeout;

use crate::QuorumView;
use crate::network_reconfig::KnownNodes;

pub(crate) mod signatures;

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
pub struct QuorumNodeJoinApproval {
    quorum_seq: SeqNo,

    requesting_node: NodeId,
    origin_node: NodeId,
}

/// A certificate composed of enough QuorumNodeJoinResponses to consider that enough
/// existing quorum nodes have accepted the new node
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumJoinCertificate {
    network_view_seq: SeqNo,
    approvals: Vec<StoredMessage<QuorumNodeJoinApproval>>,
}

/// Reason message for the rejection of quorum entering request
#[derive(Clone, Debug)]
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
    Successful(QuorumNodeJoinApproval),

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

#[derive(Clone, Serialize, Deserialize)]
pub struct NodeTriple {
    node_id: NodeId,
    node_type: NodeType,
    addr: PeerAddr,
    pub_key: Vec<u8>,
}

/// The response to the request to join the network
/// Returns the list of known nodes in the network, including the newly added node
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum NetworkJoinResponseMessage {
    Successful(Signature, KnownNodesMessage),
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
    NotNecessary,
}

#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct ReconfigurationMessage {
    seq: SeqNo,
    message_type: ReconfigurationMessageType,
}

/// Reconfiguration message type
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum ReconfigurationMessageType {
    NetworkReconfig(NetworkReconfigMessage),
    QuorumReconfig(QuorumReconfigMessage),
}

pub type NetworkJoinCert = (NodeId, Signature);

/// Network reconfiguration messages (Related only to the network view)
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum NetworkReconfigMessage {
    NetworkJoinRequest(NodeTriple),
    NetworkJoinResponse(NetworkJoinResponseMessage),
    NetworkHelloRequest(NodeTriple, Vec<NetworkJoinCert>),
    NetworkHelloReply(KnownNodesMessage)
}

/// A certificate that a given node sent a quorum view
pub type QuorumViewCert = StoredMessage<QuorumView>;

#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumReconfigMessage {
    /// A state request for the current network view
    NetworkViewStateRequest,
    /// The response to the state request
    NetworkViewState(QuorumView),
    /// A request to join the current quorum
    QuorumEnterRequest(QuorumEnterRequest),
    /// The response to the request to join the quorum, 2f+1 responses
    /// are required to consider the request successful and allow for this
    /// to be passed to the ordering protocol as a QuorumJoinCertificate
    QuorumEnterResponse(QuorumEnterResponse),
    /// A message to indicate that a node has entered the quorum
    QuorumUpdated(QuorumView),
    /// A request to leave the current quorum
    QuorumLeaveRequest(QuorumLeaveRequest),
    /// The response to the request to leave the quorum
    QuorumLeaveResponse(QuorumLeaveResponse),
}

/// Messages that will be sent via channel to the reconfiguration module
pub enum ReconfigMessage {
    TimeoutReceived(Vec<RqTimeout>)
}

impl QuorumNodeJoinApproval {
    pub fn new(network_view_seq: SeqNo, requesting_node: NodeId, origin_node: NodeId) -> Self {
        Self { quorum_seq: network_view_seq, requesting_node, origin_node }
    }
}

impl ReconfigurationMessage {
    pub fn new(seq: SeqNo, message_type: ReconfigurationMessageType) -> Self {
        Self { seq, message_type }
    }

    pub fn into_inner(self) -> (SeqNo, ReconfigurationMessageType) {
        (self.seq, self.message_type)
    }
}

impl NodeTriple {
    pub fn new(node_id: NodeId, public_key: Vec<u8>, address: PeerAddr, node_type: NodeType) -> Self {
        Self {
            node_id,
            node_type,
            addr: address,
            pub_key: public_key,
        }
    }

    pub fn node_type(&self) -> NodeType {
        self.node_type
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
        let mut known_nodes = Vec::with_capacity(value.node_info().len());

        value.node_info().iter().for_each(|(node_id, node_info)| {
            known_nodes.push(NodeTriple {
                node_id: *node_id,
                node_type: node_info.node_type(),
                addr: node_info.addr().clone(),
                pub_key: node_info.pk().pk_bytes().to_vec(),
            })
        });

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
        write!(f, "NodeTriple {{ node_id: {:?}, addr: {:?}, type: {:?}}}", self.node_id, self.addr, self.node_type)
    }
}

pub struct ReconfData;

impl Serializable for ReconfData {
    type Message = ReconfigurationMessage;

    //TODO: Implement capnproto messages
}

impl ReconfigurationProtocolMessage for ReconfData {
    type QuorumJoinCertificate = QuorumJoinCertificate;
}

impl QuorumEnterRequest {
    pub fn new(node_triple: NodeTriple) -> Self {
        Self { node_triple }
    }

    pub fn into_inner(self) -> NodeTriple {
        self.node_triple
    }
}

impl QuorumJoinCertificate {
    pub fn new(network_view_seq: SeqNo, approvals: Vec<StoredMessage<QuorumNodeJoinApproval>>) -> Self {
        Self { network_view_seq, approvals }
    }
}

impl Debug for QuorumReconfigMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {

        // Debug implementation for QuorumReconfigMessage
        match self {
            QuorumReconfigMessage::NetworkViewStateRequest => write!(f, "NetworkViewStateRequest"),
            QuorumReconfigMessage::NetworkViewState(quorum_view) => write!(f, "NetworkViewState()"),
            QuorumReconfigMessage::QuorumEnterRequest(quorum_enter_request) => write!(f, "QuorumEnterRequest()"),
            QuorumReconfigMessage::QuorumEnterResponse(quorum_enter_response) => write!(f, "QuorumEnterResponse()"),
            QuorumReconfigMessage::QuorumUpdated(quorum_view) => write!(f, "QuorumUpdated()"),
            QuorumReconfigMessage::QuorumLeaveRequest(quorum_leave_request) => write!(f, "QuorumLeaveRequest()"),
            QuorumReconfigMessage::QuorumLeaveResponse(quorum_leave_response) => write!(f, "QuorumLeaveResponse()"),
        }

    }
}

impl Orderable for QuorumNodeJoinApproval {
    fn sequence_number(&self) -> SeqNo {
        self.quorum_seq
    }
}