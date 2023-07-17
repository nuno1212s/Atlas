use std::sync::Arc;
use log::info;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use atlas_core::timeouts::Timeouts;
use crate::message::{QuorumReconfigMessage, QuorumViewCert, ReconfData};
use crate::{GeneralNodeInfo, QuorumProtocolResponse, SeqNoGen};
use crate::quorum_reconfig::node_types::client::ClientQuorumView;
use crate::quorum_reconfig::node_types::replica::ReplicaQuorumView;

pub mod client;
pub mod replica;

pub(crate) enum NodeType<JC> {
    Client(ClientQuorumView),
    Replica(ReplicaQuorumView<JC>),
}

impl<JC> NodeType<JC> {
    pub fn iterate<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(client) => {
                client.iterate(seq_no, node, network_node, timeouts)
            }
            NodeType::Replica(replica) => {
                replica.iterate(seq_no, node, network_node, timeouts)
            }
        }
    }

    fn handle_view_state_message<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, quorum_reconfig: QuorumViewCert) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(client) => {
                client.handle_view_state_message(seq_no, node, network_node, quorum_reconfig)
            }
            NodeType::Replica(replica) => {
                replica.handle_view_state_message(seq_no, node, network_node, quorum_reconfig)
            }
        }
    }

    fn handle_view_state_request_message<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, seq_no: SeqNo) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(client) => {
                client.handle_view_state_request(seq_gen, node, network_node, header)
            }
            NodeType::Replica(replica) => {
                replica.handle_view_state_request(node, network_node, header, seq_no)
            }
        }
    }

    pub fn handle_reconfigure_message<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, seq: SeqNo, quorum_reconfig: QuorumReconfigMessage) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        info!("Received a view state message from {:?} with header {:?}", header.from(), header);

        match quorum_reconfig {
            QuorumReconfigMessage::NetworkViewStateRequest => {
                return self.handle_view_state_request_message(seq_gen, node, network_node, header, seq);
            }
            QuorumReconfigMessage::NetworkViewState(view_state) => {
                return self.handle_view_state_message(seq_gen, node, network_node, StoredMessage::new(header, view_state));
            }
            QuorumReconfigMessage::QuorumEnterRequest(_) => {}
            QuorumReconfigMessage::QuorumEnterResponse(_) => {}
            QuorumReconfigMessage::QuorumUpdated(_) => {}
            QuorumReconfigMessage::QuorumLeaveRequest(_) => {}
            QuorumReconfigMessage::QuorumLeaveResponse(_) => {}
        }

        QuorumProtocolResponse::Nil
    }

    pub fn handle_timeout<NT>(&mut self, seq: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(client) => {
                client.handle_timeout(seq, node, network_node, timeouts)
            }
            NodeType::Replica(replica) => {
                replica.handle_timeout(seq, node, network_node, timeouts)
            }
        }
    }
}