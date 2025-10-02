use std::sync::Arc;

use atlas_common::error::*;
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration::NetworkInformationProvider;

use crate::log_transfer::networking::signature_ver::LogTransferVerificationHelper;
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_core::serialize::NoProtocol;

/// The abstraction for log transfer protocol messages.
/// This allows us to have any log transfer protocol work with the same backbone
pub trait LogTransferMessage<RQ, OP>: Send + Sync {
    /// The message type for the log transfer protocol
    type LogTransferMessage: SerMsg + 'static;

    /// Verify the message and return the message if it is valid
    fn verify_log_message<NI, LVH>(
        network_info: &Arc<NI>,
        header: &Header,
        message: &Self::LogTransferMessage,
    ) -> Result<()>
    where
        NI: NetworkInformationProvider,
        LVH: LogTransferVerificationHelper<RQ, OP, NI>,
        OP: OrderingProtocolMessage<RQ>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(
        builder: febft_capnp::cst_messages_capnp::cst_message::Builder,
        msg: &Self::LogTransferMessage,
    ) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(
        reader: febft_capnp::cst_messages_capnp::cst_message::Reader,
    ) -> Result<Self::LogTransferMessage>;
}

impl<RQ, P> LogTransferMessage<RQ, P> for NoProtocol {
    type LogTransferMessage = ();

    fn verify_log_message<NI, LVH>(
        _network_info: &Arc<NI>,
        _header: &Header,
        _message: &Self::LogTransferMessage,
    ) -> atlas_common::error::Result<Self::LogTransferMessage>
    where
        NI: NetworkInformationProvider,
        P: OrderingProtocolMessage<RQ>,
        LVH: LogTransferVerificationHelper<RQ, P, NI>,
    {
        Ok(())
    }

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(
        _: febft_capnp::cst_messages_capnp::cst_message::Builder,
        msg: &Self::LogTransferMessage,
    ) -> std::result::Result<()> {
        unimplemented!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(
        _: febft_capnp::cst_messages_capnp::cst_message::Reader,
    ) -> std::result::Result<Self::LogTransferMessage> {
        unimplemented!()
    }
}
