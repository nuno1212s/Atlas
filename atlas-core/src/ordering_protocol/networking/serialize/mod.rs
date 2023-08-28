use std::fmt::Debug;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use atlas_common::node_id::NodeId;
use atlas_common::error::*;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::Header;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_execution::serialize::ApplicationData;
use crate::ordering_protocol::networking::signature_ver::OrderProtocolSignatureVerificationHelper;

/// The basic methods needed for a view
pub trait NetworkView: Orderable + Clone {
    fn primary(&self) -> NodeId;

    fn quorum(&self) -> usize;

    fn quorum_members(&self) -> &Vec<NodeId>;

    fn f(&self) -> usize;

    fn n(&self) -> usize;
}

pub trait OrderProtocolLog: Orderable {
    // At the moment I only need orderable, but I might need more in the future
    fn first_seq(&self) -> Option<SeqNo>;
}

/// Proof of the order protocol message
pub trait OrderProtocolProof: Orderable {
    // At the moment I only need orderable, but I might need more in the future
}

/// We do not need a serde module since serde serialization is just done on the network level.
/// The abstraction for ordering protocol messages.
pub trait OrderingProtocolMessage<D>: Send + Sync {
    #[cfg(feature = "serialize_capnp")]
    type ViewInfo: NetworkView + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type ViewInfo: NetworkView + for<'a> Deserialize<'a> + Serialize + Send + Clone + Debug;

    /// The general protocol type for all messages in the ordering protocol
    #[cfg(feature = "serialize_capnp")]
    type ProtocolMessage: Orderable + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type ProtocolMessage: Orderable + for<'a> Deserialize<'a> + Serialize + Send + Clone + Debug;

    /// A shortcut type to messages that are going to be logged. (this is useful for situations
    /// where we don't log all message types that we send)
    #[cfg(feature = "serialize_capnp")]
    type LoggableMessage: Orderable + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type LoggableMessage: Orderable + for<'a> Deserialize<'a> + Serialize + Send + Clone + Debug;

    /// A proof of a given Sequence number in the consensus protocol
    /// This is used when requesting the latest consensus id in the state transfer protocol,
    /// in order to verify that a given consensus id is valid
    #[cfg(feature = "serialize_capnp")]
    type Proof: OrderProtocolProof + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type Proof: OrderProtocolProof + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    /// The metadata type for storing the proof in the persistent storage
    /// Since the message will be stored in the persistent storage, it needs to be serializable
    /// This should provide all the necessary final information to assemble the proof from the messages
    #[cfg(feature = "serialize_serde")]
    type ProofMetadata: Orderable + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type ProofMetadata: Orderable + Send + Clone;

    fn verify_order_protocol_message<NI, OPVH>(network_info: &Arc<NI>,
                                               header: &Header,
                                               message: Self::ProtocolMessage) -> Result<(bool, Self::ProtocolMessage)>
        where NI: NetworkInformationProvider,
              OPVH: OrderProtocolSignatureVerificationHelper<D, Self, NI>,
              D: ApplicationData, Self: Sized;

    fn verify_proof<NI, OPVH>(network_info: &Arc<NI>,
                              proof: Self::Proof) -> Result<(bool, Self::Proof)>
        where NI: NetworkInformationProvider,
              OPVH: OrderProtocolSignatureVerificationHelper<D, Self, NI>,
              D: ApplicationData, Self: Sized;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: febft_capnp::consensus_messages_capnp::protocol_message::Builder, msg: &Self::ProtocolMessage) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: febft_capnp::consensus_messages_capnp::protocol_message::Reader) -> Result<Self::ProtocolMessage>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_view_capnp(builder: febft_capnp::cst_messages_capnp::view_info::Builder, msg: &Self::ViewInfo) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_view_capnp(reader: febft_capnp::cst_messages_capnp::view_info::Reader) -> Result<Self::ViewInfo>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_proof_capnp(builder: febft_capnp::cst_messages_capnp::proof::Builder, msg: &Self::Proof) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_proof_capnp(reader: febft_capnp::cst_messages_capnp::proof::Reader) -> Result<Self::Proof>;
}

/// The messages for the stateful ordering protocol
pub trait StatefulOrderProtocolMessage<D, OPM>: Send + Sync {
    /// A type that defines the log of decisions made since the last garbage collection
    /// (In the case of BFT SMR the log is GCed after a checkpoint of the application)
    #[cfg(feature = "serialize_capnp")]
    type DecLog: OrderProtocolLog + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type DecLog: OrderProtocolLog + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    fn verify_decision_log<NI, OPVH>(network_info: &Arc<NI>, dec_log: Self::DecLog)
                                            -> Result<(bool, Self::DecLog)>
        where NI: NetworkInformationProvider,
              D: ApplicationData,
              OPM: OrderingProtocolMessage<D>,
              OPVH: OrderProtocolSignatureVerificationHelper<D, OPM, NI>,;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_declog_capnp(builder: febft_capnp::cst_messages_capnp::dec_log::Builder, msg: &Self::DecLog) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_declog_capnp(reader: febft_capnp::cst_messages_capnp::dec_log::Reader) -> Result<Self::DecLog>;
}