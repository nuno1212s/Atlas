use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_communication::FullNetworkNode;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_communication::serialize::Serializable;
use atlas_execution::serialize::ApplicationData;

use crate::messages::{ReplyMessage, SystemMessage};
use crate::serialize::{LogTransferMessage, OrderingProtocolMessage, ServiceMsg, StateTransferMessage};
use crate::smr::networking::NodeWrap;

pub enum ReplyType {
    Ordered,
    Unordered,
}

/// Trait for a network node capable of sending replies to clients
pub trait ReplyNode<D>: Send + Sync where D: ApplicationData {
    fn send(&self, reply_type: ReplyType, reply: ReplyMessage<D::Reply>, target: NodeId, flush: bool) -> Result<()>;

    fn send_signed(&self, reply_type: ReplyType, reply: ReplyMessage<D::Reply>, target: NodeId, flush: bool) -> Result<()>;

    fn broadcast(&self, reply_type: ReplyType, reply: ReplyMessage<D::Reply>, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>>;

    fn broadcast_signed(&self, reply_type: ReplyType, reply: ReplyMessage<D::Reply>, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>>;
}

impl<NT, D, P, S, L, NI, RM> ReplyNode<D> for NodeWrap<NT, D, P, S, L, NI, RM>
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + 'static,
{
    fn send(&self, reply_type: ReplyType, reply: ReplyMessage<D::Reply>, target: NodeId, flush: bool) -> Result<()> {
        let message = match reply_type {
            ReplyType::Ordered => {
                SystemMessage::OrderedReply(reply)
            }
            ReplyType::Unordered => {
                SystemMessage::UnorderedReply(reply)
            }
        };

        self.0.send(message, target, flush)
    }

    fn send_signed(&self, reply_type: ReplyType, reply: ReplyMessage<D::Reply>, target: NodeId, flush: bool) -> Result<()> {
        let message = match reply_type {
            ReplyType::Ordered => {
                SystemMessage::OrderedReply(reply)
            }
            ReplyType::Unordered => {
                SystemMessage::UnorderedReply(reply)
            }
        };

        self.0.send_signed(message, target, flush)
    }

    fn broadcast(&self, reply_type: ReplyType, reply: ReplyMessage<D::Reply>, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        let message = match reply_type {
            ReplyType::Ordered => {
                SystemMessage::OrderedReply(reply)
            }
            ReplyType::Unordered => {
                SystemMessage::UnorderedReply(reply)
            }
        };
        self.0.broadcast(message, targets)
    }

    fn broadcast_signed(&self, reply_type: ReplyType, reply: ReplyMessage<D::Reply>, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        let message = match reply_type {
            ReplyType::Ordered => {
                SystemMessage::OrderedReply(reply)
            }
            ReplyType::Unordered => {
                SystemMessage::UnorderedReply(reply)
            }
        };
        self.0.broadcast_signed(message, targets)
    }
}