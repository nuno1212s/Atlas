use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use getset::Getters;
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use atlas_common::crypto::hash::Digest;
use atlas_common::crypto::signature::Signature;
use atlas_common::crypto::threshold_crypto::thold_crypto::dkg::{Ack, DealerPart};
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration::{NetworkInformationProvider, NodeInfo};
use atlas_communication::serialization::{InternalMessageVerifier, Serializable};
use atlas_core::serialize::ReconfigurationProtocolMessage;
use atlas_core::timeouts::timeout::ModTimeout;

use crate::network_reconfig::KnownNodes;
use crate::quorum_config::QuorumView;

pub(crate) mod signatures;

/// Used to request to join the current quorum
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumEnterRequest {
    node_triple: NodeInfo,
}

/// Reason message for the rejection of quorum entering request
#[derive(Clone, Debug, PartialEq, Eq, Ord, PartialOrd)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumEnterRejectionReason {
    NotAuthorized,
    MissingValues,
    IncorrectNetworkViewSeq,
    NodeIsNotQuorumParticipant,
    FailedToObtainQuorum,
    AlreadyPartOfQuorum,
    CurrentlyReconfiguring,
}

/// A response to a network join request
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumEnterResponse {
    // We have accepted the request to join the quorum
    Successful(QuorumView),

    Rejected(QuorumEnterRejectionReason),
}

#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumLeaveRequest {
    node_triple: NodeInfo,
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
    nodes: Vec<NodeInfo>,
}

/// The response to the request to join the network
/// Returns the list of known nodes in the network, including the newly added node
#[derive(Clone, Debug)]
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

/// Reconfiguration message type
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum ReconfigurationMessage {
    // The network reconfiguration protocol messages
    NetworkReconfig(NetworkReconfigMessage),
    // Messages related to the threshold crypto protocol
    ThresholdCrypto(ThresholdMessages),
    // The messages related to the configuration of the quorum
    QuorumConfig(OperationMessage),
}

pub type NetworkJoinCert = (NodeId, Signature);

/// Messages related to the threshold cryptography protocols
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum ThresholdMessages {
    // Trigger a new distributed key generation algorithm, with the given
    // Quorum members and threshold
    TriggerDKG(ThresholdDKGArgs),
    // The ordered broadcast messages relating to the dealer part of the DKG protocol
    DkgDealer(OrderedBCastMessage<DealerPart>),
    // The ordered broadcast messages relating to the ack part of the DKG protocol
    DkgAck(OrderedBCastMessage<Vec<Ack>>),
}

/// The arguments for the distributed key generation protocol
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct ThresholdDKGArgs {
    // The quorum that should participate in the key generation protocol
    pub quorum: Vec<NodeId>,
    // The threshold
    pub threshold: usize,
    // The leader of this DKG iteration
    pub leader: NodeId,
}

/// Network reconfiguration protocol
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct NetworkReconfigMessage {
    seq: SeqNo,
    message_type: NetworkReconfigMessageType,
}

/// Network reconfiguration messages (Related only to the network view)
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum NetworkReconfigMessageType {
    NetworkJoinRequest(NodeInfo),
    NetworkJoinResponse(NetworkJoinResponseMessage),
    NetworkHelloRequest(NodeInfo, Vec<NetworkJoinCert>),
    NetworkHelloReply(KnownNodesMessage),
}

/// A certificate that a given node sent a quorum view
pub type QuorumViewCert = StoredMessage<QuorumView>;

/// Messages that will be sent via channel to the reconfiguration module
pub enum ReconfigMessage {
    TimeoutReceived(Vec<ModTimeout>),
}

/// Messages relating to the ordered broadcast protocol
/// Which is used to order the messages that are sent by the threshold cryptography protocols
///
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum OrderedBCastMessage<T> {
    // Participate on the ordered broadcast protocol with a value
    Value(T),
    //TODO: Make this verifiable such that the leader cannot
    // Invent new values that were not sent to him
    Order(Vec<T>),
    // Vote on a given order of the broadcast
    OrderVote(Digest),
}

/// A type to encapsulate all of the operation messages
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum OperationMessage {
    QuorumInfoOp(QuorumObtainInfoOpMessage),
    QuorumReconfiguration(QuorumJoinReconfMessages),
}

/// Messages related to the [QuorumObtainInformationOperation]
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumObtainInfoOpMessage {
    // Request information about the current quorum
    RequestInformationMessage,
    // A response message containing the information about the quorum
    QuorumInformationResponse(QuorumView),
}

/// Messages to be exchanged when a node is attempting to join the quorum
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumJoinReconfMessages {
    RequestJoinQuorum(QuorumView),
    // A response message by a quorum member, with the response about the request to enter
    // The quorum. With 2f + 1 of these responses, we can form a certificate
    // Which will be accepted by all correct members of the quorum
    LockedQuorumResponse(QuorumJoinResponse),
    // The node has been accepted into the quorum
    CommitQuorum(LockedQC),
    // The response to the commit quorum request, the last step in the quorum reconfiguration protocol
    CommitQuorumResponse(QuorumCommitResponse),
    // The commit certificate for the quorum.
    // Any replica that receives this, will move to the new quorum
    // And notify it's ordering protocol that it has done so.
    Decided(CommittedQC),
}

/// The locked quorum certificate, containing all of the accepts sent by 2f+1 of the replicas
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct LockedQC {
    proofs: Vec<StoredMessage<QuorumAcceptResponse>>,
}

/// The committed quorum certificate, containing all of the commits sent by 2f+1 of the replicas
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct CommittedQC {
    proofs: Vec<StoredMessage<QuorumCommitAcceptResponse>>,
}

/// The message that will be sent to the reconfiguration module
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumJoinResponse {
    // We have been accepted into the quorum
    Accepted(QuorumAcceptResponse),
    // We have been rejected to join the quorum
    Rejected(QuorumRejectionReason),
}

/// The response to the quorum commit request
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumCommitResponse {
    Accepted(QuorumCommitAcceptResponse),
    Rejected(QuorumRejectionReason),
}

/// The accept response for the quorum commit request, containing the next quorum view
#[derive(Clone, Debug, Getters)]
#[get = "pub"]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumCommitAcceptResponse {
    view: QuorumView,
    // Include the locked QC we are voting on so that we
    // Are effectively signing the already established certificate
    // Adding a big layer of security
    // It also guarantees that a replica can't commit vote for any round if that
    // Round is not yet locked.
    // TODO: This should be the signature of the locked QC by the sender
    qc: LockedQC,
}

/// The accept response for the quorum join request, containing the next quorum view
#[derive(Clone, Debug, Getters, PartialEq, Eq)]
#[get = "pub"]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumAcceptResponse {
    // The view of the quorum
    view: QuorumView,
}

/// The rejection reason for the quorum join request
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum QuorumRejectionReason {
    NotAuthorized,
    AlreadyAccepting,
    AlreadyAPartOfQuorum,
    ViewDoesNotMatch(QuorumView),
    SeqNoTooOld(SeqNo, SeqNo),
    SeqNoTooAdvanced(SeqNo, SeqNo),
    InvalidPart(String),
}

impl NetworkReconfigMessage {
    pub fn new(seq: SeqNo, message_type: NetworkReconfigMessageType) -> Self {
        Self { seq, message_type }
    }

    pub fn into_inner(self) -> (SeqNo, NetworkReconfigMessageType) {
        (self.seq, self.message_type)
    }
}

impl From<&KnownNodes> for KnownNodesMessage {
    fn from(value: &KnownNodes) -> Self {
        let mut known_nodes = Vec::with_capacity(value.node_info().len());

        value
            .node_info()
            .iter()
            .for_each(|(_node_id, node_info)| known_nodes.push(node_info.clone()));

        KnownNodesMessage { nodes: known_nodes }
    }
}

impl KnownNodesMessage {
    pub fn known_nodes(&self) -> &Vec<NodeInfo> {
        &self.nodes
    }

    pub fn into_nodes(self) -> Vec<NodeInfo> {
        self.nodes
    }
}

pub struct ReconfData;

pub struct ReconfVerifier;

impl InternalMessageVerifier<ReconfigurationMessage> for ReconfVerifier {
    fn verify_message<NI>(
        _info_provider: &Arc<NI>,
        _header: &Header,
        _message: &ReconfigurationMessage,
    ) -> atlas_common::error::Result<()>
    where
        NI: NetworkInformationProvider,
    {
        Ok(())
    }
}

impl Serializable for ReconfData {
    type Message = ReconfigurationMessage;
    type Verifier = ReconfVerifier;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(
        builder: febft_capnp::messages_capnp::system::Builder,
        msg: &Self::Message,
    ) -> atlas_common::error::Result<()> {
        todo!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(
        reader: febft_capnp::messages_capnp::system::Reader,
    ) -> atlas_common::error::Result<Self::Message> {
        todo!()
    }
}

impl Orderable for NetworkReconfigMessage {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl NetworkReconfigMessage {
    pub fn seq(&self) -> SeqNo {
        self.seq
    }
    pub fn message_type(&self) -> &NetworkReconfigMessageType {
        &self.message_type
    }
}

impl ReconfigurationProtocolMessage for ReconfData {
    type QuorumJoinCertificate = ();
}

impl QuorumEnterRequest {
    pub fn new(node_triple: NodeInfo) -> Self {
        Self { node_triple }
    }

    pub fn into_inner(self) -> NodeInfo {
        self.node_triple
    }
}

impl ThresholdDKGArgs {
    pub fn init_args(quorum: Vec<NodeId>, threshold: usize, leader: NodeId) -> Self {
        Self {
            quorum,
            threshold,
            leader,
        }
    }
}

impl QuorumAcceptResponse {
    pub fn init(view: QuorumView) -> Self {
        Self { view }
    }
}

impl QuorumCommitAcceptResponse {
    pub fn init(view: QuorumView, locked_qc: LockedQC) -> Self {
        Self {
            view,
            qc: locked_qc,
        }
    }
}

impl LockedQC {
    pub fn new(proofs: Vec<StoredMessage<QuorumAcceptResponse>>) -> Self {
        Self { proofs }
    }

    pub fn quorum(&self) -> &QuorumView {
        self.proofs.first().unwrap().message().view()
    }

    pub fn proofs(&self) -> &Vec<StoredMessage<QuorumAcceptResponse>> {
        &self.proofs
    }

    pub fn into_inner(self) -> Vec<StoredMessage<QuorumAcceptResponse>> {
        self.proofs
    }
}

impl CommittedQC {
    pub fn new(proofs: Vec<StoredMessage<QuorumCommitAcceptResponse>>) -> Self {
        Self { proofs }
    }

    pub fn quorum(&self) -> &QuorumView {
        self.proofs.first().unwrap().message().view()
    }

    pub fn proofs(&self) -> &Vec<StoredMessage<QuorumCommitAcceptResponse>> {
        &self.proofs
    }

    pub fn into_inner(self) -> Vec<StoredMessage<QuorumCommitAcceptResponse>> {
        self.proofs
    }
}

impl PartialEq for LockedQC {
    fn eq(&self, other: &Self) -> bool {
        std::iter::zip(self.proofs.iter(), other.proofs.iter()).all(|(msg_a, msg_b)| {
            *msg_a.header() == *msg_b.header() && *msg_a.message() == *msg_b.message()
        })
    }
}

impl<T> Debug for OrderedBCastMessage<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderedBCastMessage::Value(_) => {
                write!(f, "OrderedBCastMessage::Value()")
            }
            OrderedBCastMessage::Order(_) => {
                write!(f, "OrderedBCastMessage::Order()")
            }
            OrderedBCastMessage::OrderVote(vote) => {
                write!(f, "OrderedBCastMessage::OrderVote({vote:?})")
            }
        }
    }
}
