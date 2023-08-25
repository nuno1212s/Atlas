use std::ops::Deref;
use std::sync::Arc;
use atlas_common::channel::ChannelSyncTx;
use atlas_common::globals::ReadOnly;
use atlas_communication::message::StoredMessage;
use crate::messages::Protocol;
use crate::ordering_protocol::networking::serialize::OrderingProtocolMessage;

/// The message type of the channel
pub type FollowerChannelMsg<OP> = FollowerEvent<OP>;

pub enum FollowerEvent<OP: OrderingProtocolMessage> {
    ReceivedConsensusMsg(
        OP::ViewInfo,
        Arc<ReadOnly<StoredMessage<Protocol<OP::ProtocolMessage>>>>,
    ),
    ReceivedViewChangeMsg(Arc<ReadOnly<StoredMessage<Protocol<OP::ProtocolMessage>>>>),
}

/// A handle to the follower handling thread
///
/// Allows us to pass the thread notifications on what is happening so it
/// can handle the events properly
#[derive(Clone)]
pub struct FollowerHandle<OP: OrderingProtocolMessage> {
    tx: ChannelSyncTx<FollowerChannelMsg<OP>>,
}

impl<OP: OrderingProtocolMessage> FollowerHandle<OP> {
    pub fn new(tx: ChannelSyncTx<FollowerChannelMsg<OP>>) -> Self {
        FollowerHandle { tx }
    }
}

impl<OP: OrderingProtocolMessage> Deref for FollowerHandle<OP> {
    type Target = ChannelSyncTx<FollowerChannelMsg<OP>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
