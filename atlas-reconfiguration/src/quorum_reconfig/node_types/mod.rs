use std::sync::Arc;
use log::info;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use atlas_core::timeouts::Timeouts;
use crate::message::{QuorumEnterRequest, QuorumEnterResponse, QuorumReconfigMessage, QuorumViewCert, ReconfData};
use crate::{GeneralNodeInfo, QuorumProtocolResponse, SeqNoGen};
use crate::quorum_reconfig::node_types::client::ClientQuorumView;
use crate::quorum_reconfig::node_types::replica::ReplicaQuorumView;
use crate::quorum_reconfig::QuorumView;

pub mod client;
pub mod replica;

pub(crate) enum NodeType {
    Client(ClientQuorumView),
    Replica(ReplicaQuorumView),
}

impl NodeType {
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

    pub fn is_response_to_request(&self, _seq_gen: &SeqNoGen, _header: &Header, _seq: SeqNo, message: &QuorumReconfigMessage) -> bool {
        match message {
            QuorumReconfigMessage::NetworkViewState(_) => true,
            QuorumReconfigMessage::QuorumEnterResponse(_) => true,
            QuorumReconfigMessage::QuorumLeaveResponse(_) => true,
            _ => false
        }
    }

    fn handle_view_state_message<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts, quorum_reconfig: QuorumViewCert) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(client) => {
                client.handle_view_state_message(seq_no, node, network_node, quorum_reconfig)
            }
            NodeType::Replica(replica) => {
                replica.handle_view_state_message(seq_no, node, network_node, timeouts, quorum_reconfig)
            }
        }
    }

    fn handle_view_state_request_message<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, seq_no: SeqNo) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(client) => {
                client.handle_view_state_request(seq_gen, node, network_node, header, seq_no)
            }
            NodeType::Replica(replica) => {
                replica.handle_view_state_request(node, network_node, header, seq_no)
            }
        }
    }

    fn handle_enter_request<NT>(&mut self, seq: SeqNo, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: QuorumEnterRequest) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(_) => {
                QuorumProtocolResponse::Nil
            }
            NodeType::Replica(replica) => {
                replica.handle_quorum_enter_request(seq, node, network_node, header, message)
            }
        }
    }

    fn handle_enter_response<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts, header: Header, message: QuorumEnterResponse) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(_) => {
                QuorumProtocolResponse::Nil
            }
            NodeType::Replica(replica) => {
                replica.handle_quorum_enter_response(seq_gen, node, network_node, timeouts, header, message)
            }
        }
    }

    fn handle_quorum_updated<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, quorum_view: QuorumView) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self {
            NodeType::Client(client) => {
                client.handle_quorum_entered_received(seq_gen, node, network_node, StoredMessage::new(header, quorum_view))
            }
            NodeType::Replica(_) => {
                QuorumProtocolResponse::Nil
            }
        }
    }

    pub fn handle_reconfigure_message<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>,
                                          timeouts: &Timeouts,
                                          header: Header, seq: SeqNo, quorum_reconfig: QuorumReconfigMessage) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {

        info!("Received a quorum reconfig message {:?} from {:?} with header {:?}",quorum_reconfig, header.from(), header, );

        match quorum_reconfig {
            QuorumReconfigMessage::NetworkViewStateRequest => {
                return self.handle_view_state_request_message(seq_gen, node, network_node, header, seq);
            }
            QuorumReconfigMessage::NetworkViewState(view_state) => {
                return self.handle_view_state_message(seq_gen, node, network_node, timeouts, StoredMessage::new(header, view_state));
            }
            QuorumReconfigMessage::QuorumEnterRequest(request) => {
                return self.handle_enter_request(seq, node, network_node, header, request);
            }
            QuorumReconfigMessage::QuorumEnterResponse(enter_response) => {
                return self.handle_enter_response(seq_gen, node, network_node, timeouts, header, enter_response);
            }
            QuorumReconfigMessage::QuorumUpdated(quorum_update) => {
                return self.handle_quorum_updated(seq_gen, node, network_node, header, quorum_update);
            }
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