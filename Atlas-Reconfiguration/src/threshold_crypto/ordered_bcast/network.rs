use crate::message::{OrderedBCastMessage};
use atlas_common::node_id::NodeId;

pub trait OrderedBCastNode<T> {
    /// Send a participating quorum message to a specific node
    fn send_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<T>,
        target: NodeId,
    ) -> atlas_common::error::Result<()>;

    /// Broadcast a participating quorum message to a set of nodes
    fn broadcast_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<T>,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;
}
