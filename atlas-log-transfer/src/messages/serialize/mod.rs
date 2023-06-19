use std::marker::PhantomData;
use atlas_core::serialize::{LogTransferMessage, OrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_execution::serialize::ApplicationData;
use crate::messages::LTMessage;

pub struct LTMsg<D: ApplicationData, OP:OrderingProtocolMessage, SOP: StatefulOrderProtocolMessage>(PhantomData<(D, OP, SOP)>);

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