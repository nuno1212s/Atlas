use std::sync::Arc;

use crate::state_transfer::networking::signature_ver::StateTransferVerificationHelper;
use atlas_common::error::*;
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration::NetworkInformationProvider;

/// The abstraction for state transfer protocol messages.
/// This allows us to have any state transfer protocol work with the same backbone
pub trait StateTransferMessage: Send {
    type StateTransferMessage: SerMsg + Sync;

    /// Verify the message and return the message if it is valid
    fn verify_state_message<NI, SVH>(
        network_info: &Arc<NI>,
        header: &Header,
        message: Self::StateTransferMessage,
    ) -> Result<Self::StateTransferMessage>
    where
        NI: NetworkInformationProvider,
        SVH: StateTransferVerificationHelper;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(
        builder: febft_capnp::cst_messages_capnp::cst_message::Builder,
        msg: &Self::StateTransferMessage,
    ) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(
        reader: febft_capnp::cst_messages_capnp::cst_message::Reader,
    ) -> Result<Self::StateTransferMessage>;
}
