use std::cmp::Ordering;
use std::collections::{BTreeSet, VecDeque};
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use either::Either;
use log::{debug, error, info, warn};
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{InvalidSeqNo, Orderable, SeqNo, tbo_advance_message_queue, tbo_queue_message};
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use atlas_core::reconfiguration_protocol::ReconfigurableNodeTypes;
use atlas_core::timeouts::Timeouts;
use crate::message::{QuorumEnterRequest, QuorumEnterResponse, QuorumReconfigMessage, QuorumViewCert, ReconfData, ReconfigQuorumMessage, ReconfigurationMessage, ReconfigurationMessageType};
use crate::{GeneralNodeInfo, QuorumProtocolResponse, SeqNoGen};
use crate::quorum_reconfig::node_types::client::ClientQuorumView;
use crate::quorum_reconfig::node_types::replica::ReplicaQuorumView;
use crate::quorum_reconfig::QuorumView;

pub mod client;
pub mod replica;

pub(crate) struct Node {
    quorum_view: QuorumViewer,
    quorum_view_handling: QuorumViewHandling,
    node_type: NodeType,
}

pub(crate) enum NodeType {
    Client(ClientQuorumView),
    Replica(ReplicaQuorumView),
}

#[derive(Debug)]
struct QuorumViewHandling {
    curr_view: QuorumView,
    voted: BTreeSet<NodeId>,
    vote_queue: VecDeque<VecDeque<QuorumViewVote>>,
    votes: Vec<QuorumViewVotes>,
}

enum VoteProcessResult {
    None,
    NewView(QuorumView),
}

pub(crate) enum QuorumNodeResponse {
    NewView(QuorumView),
    Nil,
    Done,
    DoneWithView(QuorumView),
}

impl QuorumViewHandling {
    fn init(default: QuorumView) -> Self {
        Self {
            curr_view: default,
            voted: Default::default(),
            vote_queue: Default::default(),
            votes: Default::default(),
        }
    }

    fn votes(&self) -> usize {
        self.voted.len()
    }

    fn poll_votes(&mut self) -> Option<QuorumViewVote> {
        if let Some(vote) = self.vote_queue.front_mut() {
            if !vote.is_empty() {
                return vote.pop_back();
            }
        }

        None
    }

    fn handle_view_rcvd(&mut self, voter: NodeId, recv_view: QuorumView) -> Option<QuorumView> {
        match recv_view.sequence_number().index(self.curr_view.sequence_number()) {
            Either::Left(_) => {}
            Either::Right(0) => {
                // We don't need to handle any messages for our current quorum view seq, as it's already installed
                debug!("Received a vote for a view whose seq that is the same as our current view, ignoring")
            }
            Either::Right(1) => {
                return self.process_vote(voter, recv_view);
            }
            Either::Right(_) => {
                tbo_queue_message(self.curr_view.sequence_number(), &mut self.vote_queue, QuorumViewVote(voter, recv_view));
            }
        }

        None
    }

    fn installed_view(&mut self, quorum_view: QuorumView) {
        let old_view_seq = self.curr_view.sequence_number();

        self.curr_view = quorum_view.clone();

        match quorum_view.sequence_number().index(old_view_seq) {
            Either::Left(_) => {
                unreachable!("We should never install a view that is older than our current view")
            }
            Either::Right(index) => {
                for _ in 0..index {
                    //Clear the message queues until the most recent view
                    self.next_view();
                }
            }
        }
    }

    fn process_vote(&mut self, voter: NodeId, recv_view: QuorumView) -> Option<QuorumView> {
        if self.curr_view.quorum_members().contains(&voter) && self.voted.insert(voter) {
            if let Some(pos) = self.votes.iter().position(|vote| { vote.0 == recv_view }) {
                let votes = self.votes.get_mut(pos).unwrap();

                votes.1 += 1;
            } else {
                self.votes.push(QuorumViewVotes(recv_view, 1));
            }
        }

        self.votes.sort();

        // Because sort goes in asc order, then the quorum view in the last slot is the one with the most votes
        if let Some(view) = self.votes.last() {
            if view.1 >= necessary_votes(self.curr_view.quorum_members()) {
                let view = self.votes.pop().unwrap();

                return Some(view.0);
            }
        }

        if self.voted.len() >= self.curr_view.quorum_members().len() {
            // We have received votes from all members but they don't match?
            error!("Received votes from all quorum members but we were not able to obtain the necessary votes to move to a new view?");
        }

        None
    }

    fn next_view(&mut self) {
        self.votes.clear();
        self.voted.clear();
        tbo_advance_message_queue(&mut self.vote_queue);
    }
}

#[derive(Debug)]
struct QuorumViewVote(NodeId, QuorumView);

#[derive(Debug)]
struct QuorumViewVotes(QuorumView, usize);

impl Orderable for QuorumViewVote {
    fn sequence_number(&self) -> SeqNo {
        self.1.sequence_number()
    }
}

impl Eq for QuorumViewVotes {}

impl PartialEq<Self> for QuorumViewVotes {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl PartialOrd<Self> for QuorumViewVotes {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.1.partial_cmp(&other.1)
    }
}

impl Ord for QuorumViewVotes {
    fn cmp(&self, other: &Self) -> Ordering {
        self.1.cmp(&other.1)
    }
}

#[derive(Clone)]
pub struct QuorumViewer {
    quorum_view: Arc<RwLock<QuorumView>>,
}

impl QuorumViewer {
    pub fn view(&self) -> QuorumView {
        self.quorum_view.read().unwrap().clone()
    }

    pub fn quorum_members(&self) -> Vec<NodeId> {
        self.quorum_view.read().unwrap().quorum_members().clone()
    }

    /// This install view is direct and to prevent any concurrency issues between messages
    /// sent between protocols, etc. This should always be followed by returning [QuorumNodeResponse::NewView]
    /// So we can clean everything up
    pub fn install_view(&self, view: QuorumView) {
        *self.quorum_view.write().unwrap() = view;
    }
}

impl Orderable for QuorumViewer {
    fn sequence_number(&self) -> SeqNo {
        self.quorum_view.read().unwrap().sequence_number()
    }
}

impl Node {
    pub fn init(bootstrap_nodes: Vec<NodeId>, node_type: ReconfigurableNodeTypes, min_stable_node_count: usize) -> (Self, QuorumViewer) {
        let quorum_view = Arc::new(RwLock::new(QuorumView::with_bootstrap_nodes(bootstrap_nodes)));

        let quorum_viewer = QuorumViewer {
            quorum_view: quorum_view.clone(),
        };

        let node_type = match node_type {
            ReconfigurableNodeTypes::ClientNode(quorum_message) => {
                NodeType::Client(ClientQuorumView::new(quorum_view.clone(), quorum_message, min_stable_node_count))
            }
            ReconfigurableNodeTypes::QuorumNode(channel_tx, channel_rx) => {
                NodeType::Replica(ReplicaQuorumView::new(quorum_viewer.clone(), channel_tx, channel_rx, vec![], min_stable_node_count))
            }
        };

        (Node {
            quorum_view: quorum_viewer.clone(),
            quorum_view_handling: QuorumViewHandling {
                curr_view: quorum_viewer.view(),
                voted: Default::default(),
                vote_queue: Default::default(),
                votes: Default::default(),
            },
            node_type,
        }, quorum_viewer)
    }


    pub fn iterate<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        while let Some(vote) = self.quorum_view_handling.poll_votes() {
            match self.process_vote(vote.0, vote.1) {
                QuorumNodeResponse::NewView(view) => {
                    self.quorum_view_handling.installed_view(view);
                }
                _ => {}
            }
        }

        let result = match &mut self.node_type {
            NodeType::Client(client) => {
                client.iterate(seq_no, node, network_node, timeouts)
            }
            NodeType::Replica(replica) => {
                replica.iterate(seq_no, node, network_node, timeouts)
            }
        };

        self.translate_result(result)
    }

    fn process_vote(&mut self, voter: NodeId, view: QuorumView) -> QuorumNodeResponse {
        let current_view = self.quorum_view.view();

        if let Some(view) = self.quorum_view_handling.handle_view_rcvd(voter, view) {
            self.quorum_view.install_view(view.clone());

            return QuorumNodeResponse::NewView(view);
        }

        return QuorumNodeResponse::Nil;
    }

    pub fn is_response_to_request(&self, _seq_gen: &SeqNoGen, _header: &Header, _seq: SeqNo, message: &QuorumReconfigMessage) -> bool {
        match message {
            QuorumReconfigMessage::NetworkViewState(_) => true,
            QuorumReconfigMessage::QuorumEnterResponse(_) => true,
            QuorumReconfigMessage::QuorumLeaveResponse(_) => true,
            _ => false
        }
    }

    fn handle_view_state_message<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts, quorum_reconfig: QuorumViewCert) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.node_type {
            NodeType::Client(client) => {
                client.handle_view_state_message(seq_no, node, network_node, quorum_reconfig)
            }
            NodeType::Replica(replica) => {
                replica.handle_view_state_message(seq_no, node, network_node, timeouts, quorum_reconfig)
            }
        }
    }

    fn handle_view_state_request_message<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, seq_no: SeqNo) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.node_type {
            NodeType::Client(client) => {
                client.handle_view_state_request(seq_gen, node, network_node, header, seq_no)
            }
            NodeType::Replica(replica) => {
                replica.handle_view_state_request(node, network_node, header, seq_no)
            }
        }
    }

    fn handle_enter_request<NT>(&mut self, seq: SeqNo, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: QuorumEnterRequest) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.node_type {
            NodeType::Client(_) => {
                QuorumNodeResponse::Nil
            }
            NodeType::Replica(replica) => {
                replica.handle_quorum_enter_request(seq, node, network_node, header, message)
            }
        }
    }

    fn handle_enter_response<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts, header: Header, message: QuorumEnterResponse) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.node_type {
            NodeType::Client(_) => {
                QuorumNodeResponse::Nil
            }
            NodeType::Replica(replica) => {
                replica.handle_quorum_enter_response(seq_gen, node, network_node, timeouts, header, message)
            }
        }
    }

    fn handle_quorum_updated<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, quorum_view: QuorumView) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        self.process_vote(header.from(), quorum_view);

        QuorumNodeResponse::Nil
    }

    fn handle_quorum_reconfig_message<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, seq: SeqNo, message: ReconfigQuorumMessage) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.node_type {
            NodeType::Client(_) => {
                warn!("Client received a quorum reconfig message from {:?} with header {:?}", header.from(), header);

                QuorumNodeResponse::Nil
            }
            NodeType::Replica(replica) => {
                let message = ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumReconfig(message)));

                replica.handle_quorum_reconfig_message(node, network_node, header, message)
            }
        }
    }

    pub fn handle_reconfigure_message<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>,
                                          timeouts: &Timeouts,
                                          header: Header, seq: SeqNo, quorum_reconfig: QuorumReconfigMessage) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        debug!("Received a quorum reconfig message {:?} from {:?} with header {:?}",quorum_reconfig, header.from(), header, );

        let result = match quorum_reconfig {
            QuorumReconfigMessage::NetworkViewStateRequest => {
                self.handle_view_state_request_message(seq_gen, node, network_node, header, seq)
            }
            QuorumReconfigMessage::NetworkViewState(view_state) => {
                self.handle_view_state_message(seq_gen, node, network_node, timeouts, StoredMessage::new(header, view_state))
            }
            QuorumReconfigMessage::QuorumEnterRequest(request) => {
                self.handle_enter_request(seq, node, network_node, header, request)
            }
            QuorumReconfigMessage::QuorumEnterResponse(enter_response) => {
                self.handle_enter_response(seq_gen, node, network_node, timeouts, header, enter_response)
            }
            QuorumReconfigMessage::QuorumReconfig(reconfig_messages) => {
                self.handle_quorum_reconfig_message(node, network_node, header, seq, reconfig_messages)
            }
            QuorumReconfigMessage::QuorumUpdate(update_message) => {
                self.process_vote(header.from(), update_message)
            }
            QuorumReconfigMessage::QuorumLeaveRequest(_) => {
                QuorumNodeResponse::Nil
            }
            QuorumReconfigMessage::QuorumLeaveResponse(_) => {
                QuorumNodeResponse::Nil
            }
        };


        self.translate_result(result)
    }

    pub fn handle_timeout<NT>(&mut self, seq: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let result = match &mut self.node_type {
            NodeType::Client(client) => {
                client.handle_timeout(seq, node, network_node, timeouts)
            }
            NodeType::Replica(replica) => {
                replica.handle_timeout(seq, node, network_node, timeouts)
            }
        };

        self.translate_result(result)
    }

    fn translate_result(&mut self, result: QuorumNodeResponse) -> QuorumProtocolResponse {
        match result {
            QuorumNodeResponse::NewView(v) => {
                self.quorum_view_handling.installed_view(v);

                QuorumProtocolResponse::Nil
            }
            QuorumNodeResponse::Nil => { QuorumProtocolResponse::Nil }
            QuorumNodeResponse::Done => {
                QuorumProtocolResponse::Done
            }
            QuorumNodeResponse::DoneWithView(v) => {
                self.quorum_view_handling.installed_view(v);
                QuorumProtocolResponse::Done
            }
        }
    }
}

fn necessary_votes(vec: &Vec<NodeId>) -> usize {
    // We need at least 2/3 of the quorum to allow a node join
    let quorum_size = vec.len();

    let necessary_votes = (quorum_size as f64 / 3.0 * 2.0).ceil() as usize;

    necessary_votes
}