use std::sync::Arc;
use serde::{Deserialize, Serialize};
use atlas_communication::message::Header;
use atlas_common::error::*;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_execution::serialize::ApplicationData;
use crate::log_transfer::networking::signature_ver::LogTransferVerificationHelper;
use crate::ordering_protocol::networking::serialize::OrderingProtocolMessage;

/// The abstraction for log transfer protocol messages.
/// This allows us to have any log transfer protocol work with the same backbone
pub trait LogTransferMessage<D, OP>: Send + Sync {
    #[cfg(feature = "serialize_capnp")]
    type LogTransferMessage: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type LogTransferMessage: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    /// Verify the message and return the message if it is valid
    fn verify_log_message<NI, LVH>(network_info: &Arc<NI>,
                                          header: &Header,
                                          message: Self::LogTransferMessage) -> Result<(bool, Self::LogTransferMessage)>
        where NI: NetworkInformationProvider,
              LVH: LogTransferVerificationHelper<D, OP, NI>,
              D: ApplicationData, OP: OrderingProtocolMessage<D>;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: febft_capnp::cst_messages_capnp::cst_message::Builder, msg: &Self::LogTransferMessage) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: febft_capnp::cst_messages_capnp::cst_message::Reader) -> Result<Self::LogTransferMessage>;
}