use crate::quorum_config::network::QuorumConfigNetworkNode;
use atlas_common::node_id::NodeId;

/// The operation that propagates the quorum information to all nodes
/// Even nodes that are not part of the quorum (for example, clients)
pub struct PropagateQuorumInformationOp {
    all_known_nodes: Vec<NodeId>,

    op_state: OperationState,
}

/// The current state of the operation
pub enum OperationState {
    Waiting,
    Done,
}

impl PropagateQuorumInformationOp {
    pub fn execute<NT>(&mut self, _node: &NT)
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match self.op_state {
            OperationState::Waiting => {
                //TODO: Broadcast the information to all known nodes (even the ones not in the quorum)
                // This should really be done in a gossip like protocol to prevent a DoS by spamming the network with messages
                // But that will be for later

                self.op_state = OperationState::Done;
            }
            OperationState::Done => {}
        }
    }
}
