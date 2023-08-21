use std::sync::Arc;
use atlas_communication::message::Header;
use atlas_execution::serialize::ApplicationData;
use crate::messages::{ReplyMessage, RequestMessage, SystemMessage};
use atlas_common::error::*;
use atlas_communication::FullNetworkNode;
use atlas_communication::message_signing::NetworkMessageSignatureVerifier;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_communication::serialize::Serializable;
use crate::serialize::{LogTransferMessage, OrderingProtocolMessage, ServiceMsg, StateTransferMessage};
use crate::smr::networking::NodeWrap;

/// This is a helper trait to verify signatures of messages for the ordering protocol
pub trait OrderProtocolSignatureVerificationHelper<D, OP, NI> where D: ApplicationData, OP: OrderingProtocolMessage, NI: NetworkInformationProvider {

    /// This is a helper to verify internal player requests
    fn verify_request_message(network_info: &Arc<NI>, header: &Header, request: RequestMessage<D::Request>) -> Result<(bool, RequestMessage<D::Request>)>;

    /// Another helper to verify internal player replies
    fn verify_reply_message(network_info: &Arc<NI>, header: &Header, reply: ReplyMessage<D::Reply>) -> Result<(bool, ReplyMessage<D::Reply>)>;

    /// helper mostly to verify forwarded consensus messages, for example
    fn verify_protocol_message(network_info: &Arc<NI>, header: &Header, message: OP::ProtocolMessage) -> Result<(bool, OP::ProtocolMessage)>;
}

impl<NT, D, P, S, L, NI, RM> OrderProtocolSignatureVerificationHelper<D, P, NI> for NodeWrap<NT, D, P, S, L, NI, RM>
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + 'static, {

    fn verify_request_message(network_info: &Arc<NI>, header: &Header, request: RequestMessage<D::Request>) -> Result<(bool, RequestMessage<D::Request>)> {
        let message = SystemMessage::<D, P, S, L>::OrderedRequest(request);

        NT::NetworkSignatureVerifier::verify_signature(network_info, header, message)
            .map(|(valid, msg)| { (valid, if let SystemMessage::OrderedRequest(r) = msg { r } else { unreachable!() }) })
    }

    fn verify_reply_message(network_info: &Arc<NI>, header: &Header, reply: ReplyMessage<D::Reply>) -> Result<(bool, ReplyMessage<D::Reply>)> {
        let message = SystemMessage::<D, P, S, L>::OrderedReply(reply);

        NT::NetworkSignatureVerifier::verify_signature(network_info, header, message)
            .map(|(valid, msg)| { (valid, if let SystemMessage::OrderedReply(r) = msg { r } else { unreachable!() }) })
    }

    fn verify_protocol_message(network_info: &Arc<NI>, header: &Header, message: P::ProtocolMessage) -> Result<(bool, P::ProtocolMessage)> {
        let message = SystemMessage::<D, P, S, L>::from_protocol_message(message);

        NT::NetworkSignatureVerifier::verify_signature(network_info, header, message)
            .map(|(valid, msg)| { (valid, if let SystemMessage::ProtocolMessage(r) = msg { r.into_inner() } else { unreachable!() }) })
    }
}