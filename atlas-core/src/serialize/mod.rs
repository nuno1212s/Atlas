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
use crate::log_transfer::networking::serialize::LogTransferMessage;
use crate::log_transfer::networking::signature_ver::LogTransferVerificationHelper;

use crate::messages::{RequestMessage, SystemMessage};
use crate::messages::signature_ver::SigVerifier;
use crate::ordering_protocol::networking::serialize::{NetworkView, OrderingProtocolMessage, OrderProtocolProof};
use crate::ordering_protocol::networking::signature_ver::OrderProtocolSignatureVerificationHelper;
use crate::smr::networking::NodeWrap;
use crate::state_transfer::networking::serialize::StateTransferMessage;
use crate::state_transfer::networking::signature_ver::StateTransferVerificationHelper;

#[cfg(feature = "serialize_capnp")]
pub mod capnp;

/// Reconfiguration protocol messages
pub trait ReconfigurationProtocolMessage: Serializable + Send + Sync {
    #[cfg(feature = "serialize_capnp")]
    type QuorumJoinCertificate: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type QuorumJoinCertificate: for<'a> Deserialize<'a> + Serialize + Send + Clone;
}

/// The type that encapsulates all the serializing, so we don't have to constantly use SystemMessage
pub struct Service<D: ApplicationData, P: OrderingProtocolMessage<D>, S: StateTransferMessage, L: LogTransferMessage<D, P>>(PhantomData<(D, P, S, L)>);

pub type ServiceMessage<D: ApplicationData, P: OrderingProtocolMessage<D>, S: StateTransferMessage, L: LogTransferMessage<D, P>> = <Service<D, P, S, L> as Serializable>::Message;

pub type ClientServiceMsg<D: ApplicationData> = Service<D, NoProtocol, NoProtocol, NoProtocol>;

pub type ClientMessage<D: ApplicationData> = <ClientServiceMsg<D> as Serializable>::Message;

pub trait VerificationWrapper<M, D> where D: ApplicationData {
    // Wrap a given client request into a message
    fn wrap_request(header: Header, request: RequestMessage<D::Request>) -> M;

    fn wrap_reply(header: Header, reply: D::Reply) -> M;
}

impl<D, P, S, L> Serializable for Service<D, P, S, L> where
    D: ApplicationData + 'static, P: OrderingProtocolMessage<D> + 'static, S: StateTransferMessage + 'static, L: LogTransferMessage<D, P> + 'static {
    type Message = SystemMessage<D, P::ProtocolMessage, S::StateTransferMessage, L::LogTransferMessage>;

    fn verify_message_internal<NI, SV>(info_provider: &Arc<NI>, header: &Header, msg: &Self::Message) -> atlas_common::error::Result<bool>
        where NI: NetworkInformationProvider + 'static,
              SV: NetworkMessageSignatureVerifier<Self, NI> {
        match msg {
            SystemMessage::ProtocolMessage(protocol) => {
                let (result, message) = P::verify_order_protocol_message::<NI, SigVerifier<SV, NI, D, P, S, L>>(info_provider, header, protocol.payload().clone())?;

                Ok(result)
            }
            SystemMessage::LogTransferMessage(log_transfer) => {
                let (result, message) = L::verify_log_message::<NI, SigVerifier<SV, NI, D, P, S, L>>(info_provider, header, log_transfer.payload().clone())?;

                Ok(result)
            }
            SystemMessage::StateTransferMessage(state_transfer) => {
                let (result, message) = S::verify_state_message::<NI, SigVerifier<SV, NI, D, P, S, L>>(info_provider, header, state_transfer.payload().clone())?;

                Ok(result)
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

                let (result, message) = P::verify_order_protocol_message::<NI, SigVerifier<SV, NI, D, P, S, L>>(info_provider, message.header(), message.message().payload().clone())?;

                Ok(result)
            }
            SystemMessage::ForwardedRequestMessage(fwd_requests) => {
                let mut result = true;

                for stored_rq in fwd_requests.requests().iter() {
                    let header = stored_rq.header();
                    let message = stored_rq.message();

                    let message = SystemMessage::OrderedRequest(message.clone());

                    result &= Self::verify_message_internal::<NI, SV>(info_provider, header, &message)?;
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

impl<D> OrderingProtocolMessage<D> for NoProtocol {
    type ViewInfo = NoView;

    type ProtocolMessage = ();

    type LoggableMessage = ();

    type Proof = ();

    type ProofMetadata = ();

    fn verify_order_protocol_message<NI, OPVH>(network_info: &Arc<NI>, header: &Header, message: Self::ProtocolMessage) -> atlas_common::error::Result<(bool, Self::ProtocolMessage)> where NI: NetworkInformationProvider, OPVH: OrderProtocolSignatureVerificationHelper<D, Self, NI>, D: ApplicationData {
        Ok((false, message))
    }

    fn verify_proof<NI, OPVH>(network_info: &Arc<NI>, proof: Self::Proof) -> atlas_common::error::Result<(bool, Self::Proof)> where NI: NetworkInformationProvider, OPVH: OrderProtocolSignatureVerificationHelper<D, Self, NI>, D: ApplicationData, Self: Sized {
        Ok((false, proof))
    }

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

    fn verify_state_message<NI, SVH>(network_info: &Arc<NI>, header: &Header, message: Self::StateTransferMessage) -> atlas_common::error::Result<(bool, Self::StateTransferMessage)> where NI: NetworkInformationProvider, SVH: StateTransferVerificationHelper {
        Ok((false, message))
    }

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(_: febft_capnp::cst_messages_capnp::cst_message::Builder, msg: &Self::StateTransferMessage) -> Result<()> {
        unimplemented!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(_: febft_capnp::cst_messages_capnp::cst_message::Reader) -> Result<Self::StateTransferMessage> {
        unimplemented!()
    }
}

impl<D, P> LogTransferMessage<D, P> for NoProtocol {
    type LogTransferMessage = ();

    fn verify_log_message<NI, LVH>(network_info: &Arc<NI>, header: &Header, message: Self::LogTransferMessage) -> atlas_common::error::Result<(bool, Self::LogTransferMessage)>
        where NI: NetworkInformationProvider,
              D: ApplicationData, P: OrderingProtocolMessage<D>,
              LVH: LogTransferVerificationHelper<D, P, NI>, {
        Ok((false, message))
    }

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
