use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::Orderable;
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration::NetworkInformationProvider;
use std::fmt::Debug;
use std::sync::Arc;

/// The basic methods needed for a view
pub trait NetworkView: Orderable + Send + Clone + Debug {
    fn primary(&self) -> NodeId;

    fn quorum(&self) -> usize;

    fn quorum_members(&self) -> &Vec<NodeId>;

    fn f(&self) -> usize;

    fn n(&self) -> usize;
}

/// Proof of the order protocol message
pub trait OrderProtocolProof: Orderable {
    // At the moment I only need orderable, but I might need more in the future

    fn contained_messages(&self) -> usize;
}

pub trait PermissionedOrderingProtocolMessage: Send + Sync {
    type ViewInfo: NetworkView + SerMsg;
}

/// The signature verification helper for the ordering protocol
/// The ordering protocol always orders a RQ type, so we need something that will help us verify the signature
pub trait OrderProtocolVerificationHelper<RQ, OP, NI>: Send + Sync + 'static
where
    OP: OrderingProtocolMessage<RQ>,
{
    /// This is a helper to verify internal client requests
    fn verify_request_message(network_info: &Arc<NI>, header: &Header, request: RQ) -> Result<RQ>
    where
        NI: NetworkInformationProvider;

    /// helper mostly to verify forwarded consensus messages, for example
    fn verify_protocol_message(
        network_info: &Arc<NI>,
        header: &Header,
        message: OP::ProtocolMessage,
    ) -> Result<OP::ProtocolMessage>
    where
        NI: NetworkInformationProvider;
}

/// The protocol message trait, involving the necessary types for
/// the view transfer protocol messages
pub trait ViewTransferProtocolMessage: Send + Sync {
    /// The general protocol type for all messages in the View Transfer protocol
    type ProtocolMessage: SerMsg;

    /// Verification helper for the ordering protocol
    fn internally_verify_message<NI>(
        network_info: &Arc<NI>,
        header: &Header,
        message: &Self::ProtocolMessage,
    ) -> Result<()>
    where
        NI: NetworkInformationProvider;
}

/// We do not need a serde module since serde serialization is just done on the network level.
/// The abstraction for ordering protocol messages.
pub trait OrderingProtocolMessage<RQ>: Send + Sync + 'static {
    /// The general protocol type for all messages in the ordering protocol
    type ProtocolMessage: Orderable + SerMsg;

    /// The metadata type for storing the proof in the persistent storage
    /// Since the message will be stored in the persistent storage, it needs to be serializable
    /// This should provide all the necessary final information to assemble the proof from the messages
    type DecisionMetadata: Orderable + SerMsg;

    /// Along with the decision metadata, the
    type DecisionAdditionalInfo: SerMsg + Eq;

    /// Verification helper for the ordering protocol
    fn internally_verify_message<NI, OPVH>(
        network_info: &Arc<NI>,
        header: &Header,
        message: &Self::ProtocolMessage,
    ) -> Result<()>
    where
        NI: NetworkInformationProvider,
        OPVH: OrderProtocolVerificationHelper<RQ, Self, NI>,
        Self: Sized;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(
        builder: febft_capnp::consensus_messages_capnp::protocol_message::Builder,
        msg: &Self::ProtocolMessage,
    ) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(
        reader: febft_capnp::consensus_messages_capnp::protocol_message::Reader,
    ) -> Result<Self::ProtocolMessage>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_view_capnp(
        builder: febft_capnp::cst_messages_capnp::view_info::Builder,
        msg: &Self::ViewInfo,
    ) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_view_capnp(
        reader: febft_capnp::cst_messages_capnp::view_info::Reader,
    ) -> Result<Self::ViewInfo>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_proof_capnp(
        builder: febft_capnp::cst_messages_capnp::proof::Builder,
        msg: &Self::Proof,
    ) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_proof_capnp(
        reader: febft_capnp::cst_messages_capnp::proof::Reader,
    ) -> Result<Self::Proof>;
}
