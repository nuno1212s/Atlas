pub(super) mod active_connections;

use atlas_common::channel::oneshot::OneShotRx;
use atlas_common::node_id::NodeId;
use std::error::Error;
use std::sync::Arc;

pub type IndividualConnectionResult<E> = OneShotRx<Result<(), E>>;

/// The byte level connection controller definitions
///
/// This defines the necessary methods to provide connections to other peers
pub trait NetworkConnectionController: Send + Sync {
    type IndConnError: Error;

    type ConnectionError: Error;

    /// Check if we are connected to a given node
    fn has_connection(&self, node: &NodeId) -> bool;

    // Get the amount of nodes we are currently connected to
    fn currently_connected_node_count(&self) -> usize;

    /// Get the nodes that we are currently connected to
    fn currently_connected_nodes(&self) -> Vec<NodeId>;

    /// Connect to a given node
    fn connect_to_node(
        self: &Arc<Self>,
        node: NodeId,
    ) -> Result<Vec<IndividualConnectionResult<Self::IndConnError>>, Self::ConnectionError>;

    // Destroy our connection to a given node
    fn disconnect_from_node(self: &Arc<Self>, node: &NodeId) -> Result<(), Self::ConnectionError>;
}
