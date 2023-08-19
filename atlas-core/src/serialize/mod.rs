use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use log::info;

#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::{Header};
use atlas_communication::message_signing::NetworkMessageSignatureVerifier;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_communication::serialize::{Buf, Serializable};
use atlas_execution::serialize::ApplicationData;

use crate::messages::{RequestMessage, SystemMessage};

#[cfg(feature = "serialize_capnp")]
pub mod capnp;

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

pub trait OrderProtocolProof: Orderable {

    // At the moment I only need orderable, but I might need more in the future
}

/// A trait that indicates that a given message can be "recursively" verifiable
/// so we can verify the messages contained within
pub trait InternallyVerifiable<M> {
    /// Internally verify
    fn verify_internal_message<S, SV, NI>(network_info: &Arc<NI>, header: &Header, msg: &M) -> atlas_common::error::Result<bool>
        where S: Serializable,
              SV: NetworkMessageSignatureVerifier<S, NI>,
              NI: NetworkInformationProvider;
}

/// We do not need a serde module since serde serialization is just done on the network level.
/// The abstraction for ordering protocol messages.
pub trait OrderingProtocolMessage: Send + Sync + InternallyVerifiable<Self::ProtocolMessage> {
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

/// The abstraction for log transfer protocol messages.
/// This allows us to have any log transfer protocol work with the same backbone
pub trait LogTransferMessage: Send + Sync + InternallyVerifiable<Self::LogTransferMessage> {
    #[cfg(feature = "serialize_capnp")]
    type LogTransferMessage: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type LogTransferMessage: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: febft_capnp::cst_messages_capnp::cst_message::Builder, msg: &Self::LogTransferMessage) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: febft_capnp::cst_messages_capnp::cst_message::Reader) -> Result<Self::LogTransferMessage>;
}

/// The abstraction for state transfer protocol messages.
/// This allows us to have any state transfer protocol work with the same backbone
pub trait StateTransferMessage: Send + Sync + InternallyVerifiable<Self::StateTransferMessage> {
    #[cfg(feature = "serialize_capnp")]
    type StateTransferMessage: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type StateTransferMessage: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: febft_capnp::cst_messages_capnp::cst_message::Builder, msg: &Self::StateTransferMessage) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: febft_capnp::cst_messages_capnp::cst_message::Reader) -> Result<Self::StateTransferMessage>;
}

/// The messages for the stateful ordering protocol
pub trait StatefulOrderProtocolMessage: Send + Sync {
    /// A type that defines the log of decisions made since the last garbage collection
    /// (In the case of BFT SMR the log is GCed after a checkpoint of the application)
    #[cfg(feature = "serialize_capnp")]
    type DecLog: OrderProtocolLog + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type DecLog: OrderProtocolLog + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_declog_capnp(builder: febft_capnp::cst_messages_capnp::dec_log::Builder, msg: &Self::DecLog) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_declog_capnp(reader: febft_capnp::cst_messages_capnp::dec_log::Reader) -> Result<Self::DecLog>;
}

/// Reconfiguration protocol messages
pub trait ReconfigurationProtocolMessage: Serializable + Send + Sync {
    #[cfg(feature = "serialize_capnp")]
    type QuorumJoinCertificate: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type QuorumJoinCertificate: for<'a> Deserialize<'a> + Serialize + Send + Clone;
}

/// The type that encapsulates all the serializing, so we don't have to constantly use SystemMessage
pub struct ServiceMsg<D: ApplicationData, P: OrderingProtocolMessage, S: StateTransferMessage, L: LogTransferMessage>(PhantomData<(D, P, S, L)>);

pub type ServiceMessage<D: ApplicationData, P: OrderingProtocolMessage, S: StateTransferMessage, L: LogTransferMessage> = <ServiceMsg<D, P, S, L> as Serializable>::Message;

pub type ClientServiceMsg<D: ApplicationData> = ServiceMsg<D, NoProtocol, NoProtocol, NoProtocol>;

pub type ClientMessage<D: ApplicationData> = <ClientServiceMsg<D> as Serializable>::Message;

pub trait VerificationWrapper<M, D> where D: ApplicationData {
    // Wrap a given client request into a message
    fn wrap_request(header: Header, request: RequestMessage<D::Request>) -> M;

    fn wrap_reply(header: Header, reply: D::Reply) -> M;
}

impl<D: ApplicationData, P: OrderingProtocolMessage, S: StateTransferMessage, L: LogTransferMessage> Serializable for ServiceMsg<D, P, S, L> {
    type Message = SystemMessage<D, P::ProtocolMessage, S::StateTransferMessage, L::LogTransferMessage>;

    fn verify_message_internal<SV, NI>(info_provider: &Arc<NI>, header: &Header, msg: &Self::Message, full_raw_msg: &Buf) -> atlas_common::error::Result<bool>
        where NI: NetworkInformationProvider,
              SV: NetworkMessageSignatureVerifier<Self, NI> {
        match msg {
            SystemMessage::ProtocolMessage(protocol) => {
                P::verify_internal_message::<Self, SV, NI>(info_provider, header, protocol.payload())
            }
            SystemMessage::LogTransferMessage(log_transfer) => {
                L::verify_internal_message::<Self, SV, NI>(info_provider, header, log_transfer.payload())
            }
            SystemMessage::StateTransferMessage(state_transfer) => {
                S::verify_internal_message::<Self, SV, NI>(info_provider, header, state_transfer.payload())
            }
            SystemMessage::OrderedRequest(request) => {
                Ok(true)
            }
            SystemMessage::OrderedReply(reply) => {
                Ok(true)
            }
            SystemMessage::UnorderedReply(reply) => {
                Ok(true)
            }
            SystemMessage::UnorderedRequest(request) => {
                Ok(true)
            }
            SystemMessage::ForwardedProtocolMessage(fwd_protocol) => {
                let header = fwd_protocol.header();
                let message = fwd_protocol.message();

                Ok(true)
            }
            SystemMessage::ForwardedRequestMessage(fwd_requests) => {
                let mut result = true;

                for stored_rq in fwd_requests.requests().iter() {
                    let header = stored_rq.header();
                    let message = stored_rq.message();

                    let message = SystemMessage::OrderedRequest(message.clone());

                    result &= Self::verify_message_internal::<SV, NI>(info_provider, header, &message, full_raw_msg)?;
                }

                Ok(result)
            }
        }
    }

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: febft_capnp::messages_capnp::system::Builder, msg: &Self::Message) -> Result<()> {
        capnp::serialize_message::<D, P, S, L>(builder, msg)
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: febft_capnp::messages_capnp::system::Reader) -> Result<Self::Message> {
        capnp::deserialize_message::<D, P, S, L>(reader)
    }
}

#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Debug)]
pub struct NoProtocol;

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct NoView;

impl Orderable for NoView {
    fn sequence_number(&self) -> SeqNo {
        unimplemented!()
    }
}

impl NetworkView for NoView {
    fn primary(&self) -> NodeId {
        unimplemented!()
    }

    fn quorum(&self) -> usize {
        unimplemented!()
    }

    fn quorum_members(&self) -> &Vec<NodeId> {
        unimplemented!()
    }

    fn f(&self) -> usize {
        unimplemented!()
    }

    fn n(&self) -> usize {
        unimplemented!()
    }
}

impl InternallyVerifiable<()> for NoProtocol {
    fn verify_internal_message<S, SV, NI>(network_info: &Arc<NI>, header: &Header, msg: &()) -> atlas_common::error::Result<bool>
        where S: Serializable, SV: NetworkMessageSignatureVerifier<S, NI>, NI: NetworkInformationProvider {
        Ok(true)
    }
}

impl OrderingProtocolMessage for NoProtocol {
    type ViewInfo = NoView;

    type ProtocolMessage = ();

    type LoggableMessage = ();

    type Proof = ();

    type ProofMetadata = ();

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(_: febft_capnp::consensus_messages_capnp::protocol_message::Builder, _: &Self::ProtocolMessage) -> Result<()> {
        unimplemented!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(_: febft_capnp::consensus_messages_capnp::protocol_message::Reader) -> Result<Self::ProtocolMessage> {
        unimplemented!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn serialize_view_capnp(_: febft_capnp::cst_messages_capnp::view_info::Builder, msg: &Self::ViewInfo) -> Result<()> {
        unimplemented!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_view_capnp(_: febft_capnp::cst_messages_capnp::view_info::Reader) -> Result<Self::ViewInfo> {
        unimplemented!()
    }
}

impl StateTransferMessage for NoProtocol {
    type StateTransferMessage = ();

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(_: febft_capnp::cst_messages_capnp::cst_message::Builder, msg: &Self::StateTransferMessage) -> Result<()> {
        unimplemented!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(_: febft_capnp::cst_messages_capnp::cst_message::Reader) -> Result<Self::StateTransferMessage> {
        unimplemented!()
    }
}

impl LogTransferMessage for NoProtocol {
    type LogTransferMessage = ();

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(_: febft_capnp::cst_messages_capnp::cst_message::Builder, msg: &Self::LogTransferMessage) -> Result<()> {
        unimplemented!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(_: febft_capnp::cst_messages_capnp::cst_message::Reader) -> Result<Self::LogTransferMessage> {
        unimplemented!()
    }
}

impl OrderProtocolProof for () {}
