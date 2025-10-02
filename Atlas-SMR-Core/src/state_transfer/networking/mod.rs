use std::collections::BTreeMap;

use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_communication::message::{SerializedMessage, StoredSerializedMessage};

use crate::state_transfer::networking::serialize::StateTransferMessage;

pub mod serialize;
pub mod signature_ver;

pub trait StateTransferSendNode<STM>
where
    STM: StateTransferMessage,
{
    fn id(&self) -> NodeId;

    /// Sends a message to a given target.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send(&self, message: STM::StateTransferMessage, target: NodeId, flush: bool) -> Result<()>;

    /// Sends a signed message to a given target
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send_signed(
        &self,
        message: STM::StateTransferMessage,
        target: NodeId,
        flush: bool,
    ) -> Result<()>;

    /// Broadcast a message to all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast(
        &self,
        message: STM::StateTransferMessage,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;

    /// Broadcast a signed message for all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_signed(
        &self,
        message: STM::StateTransferMessage,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;

    /// Serialize a message to a given target.
    /// Creates the serialized byte buffer along with the header, so we can send it later.
    fn serialize_digest_message(
        &self,
        message: STM::StateTransferMessage,
    ) -> Result<(SerializedMessage<STM::StateTransferMessage>, Digest)>;

    /// Broadcast the serialized messages provided.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<STM::StateTransferMessage>>,
    ) -> std::result::Result<(), Vec<NodeId>>;
}
