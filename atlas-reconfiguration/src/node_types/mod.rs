use std::sync::Arc;
use log::info;
use atlas_common::node_id::NodeId;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use crate::message::{QuorumReconfigMessage, ReconfData};
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

    pub fn handle_view_state_message<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, quorum_reconfig: QuorumReconfigMessage) {

        info!("Received a view state message from {:?} with header {:?}", header.from(), header);

    }

}