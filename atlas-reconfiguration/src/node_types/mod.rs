use std::sync::Arc;
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use crate::message::ReconfData;
use crate::node_types::client::ClientQuorumView;
use crate::node_types::replica::ReplicaQuorumView;
use crate::GeneralNodeInfo;

pub mod client;
pub mod replica;

pub(crate) enum NodeType<JC> {
    Client(ClientQuorumView),
    Replica(ReplicaQuorumView<JC>),
}

impl<JC> NodeType<JC> {
    pub fn new_client() -> Self {
        NodeType::Client(ClientQuorumView::new())
    }

    pub fn new_replica() -> Self {
        NodeType::Replica(ReplicaQuorumView::new())
    }

    pub fn iterate<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(client) => {
                client.iterate(node, network_node)
            }
            NodeType::Replica(replica) => {
                replica.iterate(node, network_node)
            }
        }
    }
}