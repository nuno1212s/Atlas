use std::sync::Arc;
use atlas_common::error::*;
use serde::{Deserialize, Serialize};
use atlas_communication::message::Header;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use crate::state_transfer::networking::signature_ver::StateTransferVerificationHelper;

/// The abstraction for state transfer protocol messages.
/// This allows us to have any state transfer protocol work with the same backbone
pub trait StateTransferMessage: Send + Sync  {
    #[cfg(feature = "serialize_capnp")]
    type StateTransferMessage: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type StateTransferMessage: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    /// Verify the message and return the message if it is valid
    fn verify_state_message<NI, SVH>(network_info: &Arc<NI>,
                                          header: &Header,
                                          message: Self::StateTransferMessage) -> Result<(bool, Self::StateTransferMessage)>
        where NI: NetworkInformationProvider, SVH: StateTransferVerificationHelper;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: febft_capnp::cst_messages_capnp::cst_message::Builder, msg: &Self::StateTransferMessage) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: febft_capnp::cst_messages_capnp::cst_message::Reader) -> Result<Self::StateTransferMessage>;
}
