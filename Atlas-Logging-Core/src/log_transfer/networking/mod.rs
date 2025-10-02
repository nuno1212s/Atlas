use std::collections::BTreeMap;
use std::ops::Deref;

#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use crate::log_transfer::networking::serialize::LogTransferMessage;

use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;

use atlas_communication::message::{SerializedMessage, StoredSerializedMessage};

pub mod serialize;
pub mod signature_ver;

///
/// Log transfer messages
///
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub struct LogTransfer<P> {
    payload: P,
}

impl<P> LogTransfer<P> {
    pub fn new(payload: P) -> Self {
        Self { payload }
    }

    pub fn payload(&self) -> &P {
        &self.payload
    }

    pub fn into_inner(self) -> P {
        self.payload
    }
}

impl<P> Deref for LogTransfer<P> {
    type Target = P;

    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}

/// A node abstraction to
pub trait LogTransferSendNode<RQ, OP, LPM>: Send + Sync
where
    LPM: LogTransferMessage<RQ, OP>,
{
    /// Our own ID
    fn id(&self) -> NodeId;

    /// Sends a message to a given target.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send(&self, message: LPM::LogTransferMessage, target: NodeId, flush: bool) -> Result<()>;

    /// Sends a signed message to a given target
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send_signed(
        &self,
        message: LPM::LogTransferMessage,
        target: NodeId,
        flush: bool,
    ) -> Result<()>;

    /// Broadcast a message to all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast(
        &self,
        message: LPM::LogTransferMessage,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;

    /// Broadcast a signed message for all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_signed(
        &self,
        message: LPM::LogTransferMessage,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;

    /// Serialize a message to a given target.
    /// Creates the serialized byte buffer along with the header, so we can send it later.
    fn serialize_digest_message(
        &self,
        message: LPM::LogTransferMessage,
    ) -> Result<(SerializedMessage<LPM::LogTransferMessage>, Digest)>;

    /// Broadcast the serialized messages provided.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<LPM::LogTransferMessage>>,
    ) -> std::result::Result<(), Vec<NodeId>>;
}
