use crate::messages::Protocol;
use crate::ordering_protocol::networking::serialize::{
    OrderingProtocolMessage, PermissionedOrderingProtocolMessage,
};
use atlas_common::channel::sync::ChannelSyncTx;
use atlas_common::globals::ReadOnly;
use atlas_communication::message::StoredMessage;
use std::ops::Deref;
use std::sync::Arc;

/// The message type of the channel
pub type FollowerChannelMsg<RQ, OP, POP> = FollowerEvent<RQ, OP, POP>;

pub enum FollowerEvent<
    RQ,
    OP: OrderingProtocolMessage<RQ>,
    POP: PermissionedOrderingProtocolMessage,
> {
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
pub struct FollowerHandle<
    RQ,
    OP: OrderingProtocolMessage<RQ>,
    POP: PermissionedOrderingProtocolMessage,
> {
    tx: ChannelSyncTx<FollowerChannelMsg<RQ, OP, POP>>,
}

impl<RQ, OP: OrderingProtocolMessage<RQ>, POP: PermissionedOrderingProtocolMessage>
    FollowerHandle<RQ, OP, POP>
{
    pub fn new(tx: ChannelSyncTx<FollowerChannelMsg<RQ, OP, POP>>) -> Self {
        FollowerHandle { tx }
    }
}

impl<RQ, OP: OrderingProtocolMessage<RQ>, POP: PermissionedOrderingProtocolMessage> Deref
    for FollowerHandle<RQ, OP, POP>
{
    type Target = ChannelSyncTx<FollowerChannelMsg<RQ, OP, POP>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
