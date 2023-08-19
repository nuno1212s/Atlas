use std::marker::PhantomData;
use std::sync::Arc;
use atlas_communication::message::Header;
use atlas_communication::message_signing::NetworkMessageSignatureVerifier;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_communication::serialize::Serializable;
use atlas_core::serialize::{InternallyVerifiable, LogTransferMessage, OrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_execution::serialize::ApplicationData;
use crate::messages::{LogTransferMessageKind, LTMessage};

pub struct LTMsg<D: ApplicationData, OP:OrderingProtocolMessage, SOP: StatefulOrderProtocolMessage>(PhantomData<(D, OP, SOP)>);

impl<D: ApplicationData, OP: OrderingProtocolMessage, SOP: StatefulOrderProtocolMessage> InternallyVerifiable<LTMessage<OP::ViewInfo, OP::Proof, SOP::DecLog>> for LTMsg<D, OP, SOP> {
    fn verify_internal_message<S, SV, NI>(network_info: &Arc<NI>, header: &Header, msg: &LTMessage<OP::ViewInfo, OP::Proof, SOP::DecLog>) -> atlas_common::error::Result<bool>
        where S: Serializable,
              SV: NetworkMessageSignatureVerifier<S, NI>,
              NI: NetworkInformationProvider {
        match msg.kind() {
            LogTransferMessageKind::RequestLogState => {}
            LogTransferMessageKind::ReplyLogState(_, _) => {}
            LogTransferMessageKind::RequestProofs(_) => {}
            LogTransferMessageKind::ReplyLogParts(_, _) => {}
            LogTransferMessageKind::RequestLog => {}
            LogTransferMessageKind::ReplyLog(_, _) => {}
        }

        Ok(true)
    }
}

impl<D: ApplicationData, OP:OrderingProtocolMessage, SOP: StatefulOrderProtocolMessage> LogTransferMessage for LTMsg<D, OP, SOP> {

    type LogTransferMessage = LTMessage<OP::ViewInfo, OP::Proof, SOP::DecLog>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: atlas_capnp::lt_messages_capnp::lt_message::Builder, msg: &Self::LogTransferMessage) -> atlas_common::error::Result<()> {
        todo!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: atlas_capnp::lt_messages_capnp::lt_message::Reader) -> atlas_common::error::Result<Self::LogTransferMessage> {
        todo!()
    }
}