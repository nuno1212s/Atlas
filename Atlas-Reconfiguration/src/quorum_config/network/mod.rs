use std::sync::Arc;

use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_communication::stub::{ModuleOutgoingStub, RegularNetworkStub};

use crate::message::{OperationMessage, ReconfData, ReconfigurationMessage};

pub trait QuorumConfigNetworkNode {
    /// Send a participating quorum message to a specific node
    fn send_quorum_config_message(&self, message: OperationMessage, target: NodeId) -> Result<()>;

    /// Broadcast a participating quorum message to a set of nodes
    fn broadcast_quorum_message(
        &self,
        message: OperationMessage,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;
}

pub struct QuorumConfigNetworkWrapper<NT> {
    node: Arc<NT>,
}

impl<NT> QuorumConfigNetworkNode for QuorumConfigNetworkWrapper<NT>
where
    NT: RegularNetworkStub<ReconfData> + 'static,
{
    fn send_quorum_config_message(&self, message: OperationMessage, target: NodeId) -> Result<()> {
        let reconf_message = ReconfigurationMessage::QuorumConfig(message);

        self.node
            .outgoing_stub()
            .send_signed(reconf_message, target, true)
    }

    fn broadcast_quorum_message(
        &self,
        message: OperationMessage,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let reconf_message = ReconfigurationMessage::QuorumConfig(message);

        self.node
            .outgoing_stub()
            .broadcast_signed(reconf_message, target)
    }
}

impl<NT> From<Arc<NT>> for QuorumConfigNetworkWrapper<NT>
where
    NT: RegularNetworkStub<ReconfData> + 'static,
{
    fn from(node: Arc<NT>) -> Self {
        Self { node }
    }
}
