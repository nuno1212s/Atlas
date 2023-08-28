use std::ops::Deref;
use std::sync::Arc;
use atlas_common::channel::ChannelSyncTx;
use atlas_common::globals::ReadOnly;
use atlas_communication::message::StoredMessage;
use crate::messages::Protocol;
use crate::ordering_protocol::networking::serialize::OrderingProtocolMessage;

/// The message type of the channel
pub type FollowerChannelMsg<D, OP> = FollowerEvent<D, OP>;

pub enum FollowerEvent<D, OP: OrderingProtocolMessage<D>> {
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
pub struct FollowerHandle<D, OP: OrderingProtocolMessage<D>> {
    tx: ChannelSyncTx<FollowerChannelMsg<D, OP>>,
}

impl<D, OP: OrderingProtocolMessage<D>> FollowerHandle<D, OP> {
    pub fn new(tx: ChannelSyncTx<FollowerChannelMsg<D, OP>>) -> Self {
        FollowerHandle { tx }
    }
}

impl<D, OP: OrderingProtocolMessage<D>> Deref for FollowerHandle<D, OP> {
    type Target = ChannelSyncTx<FollowerChannelMsg<D, OP>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
