use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_communication::FullNetworkNode;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_communication::serialize::Serializable;
use atlas_execution::serialize::ApplicationData;
use crate::log_transfer::networking::serialize::LogTransferMessage;

use crate::messages::{ReplyMessage, SystemMessage};
use crate::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use crate::serialize::Service;
use crate::smr::networking::NodeWrap;
use crate::state_transfer::networking::serialize::StateTransferMessage;

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
          P: OrderingProtocolMessage<D> + 'static,
          L: LogTransferMessage<D, P> + 'static,
          S: StateTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, Service<D, P, S, L>> + 'static,
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