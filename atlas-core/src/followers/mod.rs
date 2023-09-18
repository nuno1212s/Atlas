use std::ops::Deref;
use std::sync::Arc;
use atlas_common::channel::ChannelSyncTx;
use atlas_common::globals::ReadOnly;
use atlas_communication::message::StoredMessage;
use crate::messages::Protocol;
use crate::ordering_protocol::networking::serialize::{OrderingProtocolMessage, PermissionedOrderingProtocolMessage};

/// The message type of the channel
pub type FollowerChannelMsg<D, OP, POP> = FollowerEvent<D, OP, POP>;

pub enum FollowerEvent<D, OP: OrderingProtocolMessage<D>, POP: PermissionedOrderingProtocolMessage> {
    ReceivedConsensusMsg(
        POP::ViewInfo,
        Arc<ReadOnly<StoredMessage<Protocol<OP::ProtocolMessage>>>>,
    ),
    ReceivedViewChangeMsg(Arc<ReadOnly<StoredMessage<Protocol<OP::ProtocolMessage>>>>),
}

/// A handle to the follower handling thread
///
/// Allows us to pass the thread notifications on what is happening so it
/// can handle the events properly
#[derive(Clone)]
pub struct FollowerHandle<D, OP: OrderingProtocolMessage<D>, POP: PermissionedOrderingProtocolMessage> {
    tx: ChannelSyncTx<FollowerChannelMsg<D, OP, POP>>,
}

impl<D, OP: OrderingProtocolMessage<D>, POP: PermissionedOrderingProtocolMessage> FollowerHandle<D, OP, POP> {
    pub fn new(tx: ChannelSyncTx<FollowerChannelMsg<D, OP, POP>>) -> Self {
        FollowerHandle { tx }
    }
}

impl<D, OP: OrderingProtocolMessage<D>, POP: PermissionedOrderingProtocolMessage> Deref for FollowerHandle<D, OP, POP> {
    type Target = ChannelSyncTx<FollowerChannelMsg<D, OP, POP>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
