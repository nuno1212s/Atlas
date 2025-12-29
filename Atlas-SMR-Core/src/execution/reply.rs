use atlas_common::error;
use atlas_common::node_id::NodeId;

#[derive(Clone, Copy)]
pub enum RequestType {
    Ordered,
    Unordered,
}

/// Trait for a network node capable of sending replies to clients
pub trait ReplyNode<RP>: Send + Sync {
    fn send(&self, reply_type: RequestType, reply: RP, target: NodeId, flush: bool) -> error::Result<()>;

    fn send_signed(
        &self,
        reply_type: RequestType,
        reply: RP,
        target: NodeId,
        flush: bool,
    ) -> error::Result<()>;

    fn broadcast(
        &self,
        reply_type: RequestType,
        reply: RP,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;

    fn broadcast_signed(
        &self,
        reply_type: RequestType,
        reply: RP,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;
}