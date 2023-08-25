use std::marker::PhantomData;
use std::sync::Arc;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;

use atlas_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_core::log_transfer::networking::signature_ver::LogTransferVerificationHelper;
use atlas_core::ordering_protocol::networking::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_execution::serialize::ApplicationData;

use crate::messages::LTMessage;

pub struct LTMsg<D: ApplicationData, OP:OrderingProtocolMessage, SOP: StatefulOrderProtocolMessage>(PhantomData<(D, OP, SOP)>);

impl<D: ApplicationData, OP:OrderingProtocolMessage, SOP: StatefulOrderProtocolMessage> LogTransferMessage for LTMsg<D, OP, SOP> {

    type LogTransferMessage = LTMessage<OP::ViewInfo, OP::Proof, SOP::DecLog>;

    fn verify_log_message<NI, LVH, D2, OP2>(network_info: &Arc<NI>, header: &Header, message: Self::LogTransferMessage) -> atlas_common::error::Result<(bool, Self::LogTransferMessage)>
        where NI: NetworkInformationProvider, LVH: LogTransferVerificationHelper<D2, OP2, NI>, D2: ApplicationData, OP2: OrderingProtocolMessage {

        Ok((true, message))
    }

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: atlas_capnp::lt_messages_capnp::lt_message::Builder, msg: &Self::LogTransferMessage) -> atlas_common::error::Result<()> {
        todo!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: atlas_capnp::lt_messages_capnp::lt_message::Reader) -> atlas_common::error::Result<Self::LogTransferMessage> {
        todo!()
    }
}