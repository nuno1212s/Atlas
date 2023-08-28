use std::cmp::Ordering;
use std::collections::{BinaryHeap, BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, RwLock};
use either::Either;
use futures::future::join_all;

use log::{debug, error, info, warn};

use atlas_common::async_runtime as rt;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::ordering::{InvalidSeqNo, Orderable, SeqNo, tbo_advance_message_queue, tbo_pop_message, tbo_queue_message};
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use atlas_core::reconfiguration_protocol::{AlterationFailReason, QuorumAlterationResponse, QuorumAttemptJoinResponse, QuorumReconfigurationMessage, QuorumReconfigurationResponse};
use atlas_core::timeouts::Timeouts;

use crate::{GeneralNodeInfo, SeqNoGen, TIMEOUT_DUR};
use crate::message::{NodeTriple, QuorumEnterRejectionReason, QuorumEnterRequest, QuorumEnterResponse, QuorumReconfigMessage, QuorumViewCert, ReconfData, ReconfigQuorumMessage, ReconfigurationMessage, ReconfigurationMessageType};
use crate::quorum_reconfig::{QuorumPredicate, QuorumView};
use crate::quorum_reconfig::node_types::{necessary_votes, QuorumNodeResponse, QuorumViewer};

struct TboQueue {
    curr_seq: SeqNo,

    joining_message: VecDeque<VecDeque<StoredMessage<ReconfigurationMessage>>>,
    confirm_join_message: VecDeque<VecDeque<StoredMessage<ReconfigurationMessage>>>,
}

/// The state of a replica
#[derive(Debug)]
enum JoiningReplicaState {
    Init,
    /// We are initializing our state so we know who to contact in order to join the quorum
    Initializing(usize, BTreeSet<NodeId>, BTreeMap<Digest, Vec<QuorumViewCert>>),
    /// We have initialized our state and are waiting for the response of the ordering protocol
    InitializedWaitingForOrderProtocol,
    /// We are currently joining the quorum
    JoiningQuorum(usize, Option<QuorumView>, BTreeMap<QuorumEnterRejectionReason, usize>, BTreeSet<NodeId>),
    /// We have been approved by the reconfiguration protocol and we are now waiting
    /// for the order protocol to respond
    JoinedQuorumWaitingForOrderProtocol,
    /// We are currently part of the quorum and are stable
    Stable,
}

/// The node state of a quorum node, waiting for new nodes to join
#[derive(Debug)]
enum QuorumNodeState {
    Init,
    /// We have received a join request from a node wanting to join and
    /// have broadcast our joining message to the quorum
    Joining(BTreeMap<NodeId, usize>, BTreeSet<NodeId>),
    /// The same as the joining state except we have already voted for a given
    /// node to join
    JoiningVoted(BTreeMap<NodeId, usize>, BTreeSet<NodeId>),
    /// We have received a quorum set of join messages and have thus broadcast
    /// our confirmation message to the quorum
    ConfirmingJoin(NodeId, BTreeSet<NodeId>),
    /// We have received quorum configuration messages
    WaitingQuorum(NodeId),
}

/// Results from polling our tbo queue
enum QuorumNodePollResult {
    None,
    Execute(StoredMessage<ReconfigurationMessage>),
}

/// The replica's quorum view and all of the necessary states
pub(crate) struct ReplicaQuorumView {
    /// The current state of the replica
    current_state: JoiningReplicaState,
    /// Queue for ordering reconfiguration messages (to handle asynchronous networks)
    tbo_queue: TboQueue,
    /// The current state of nodes joining
    join_node_state: QuorumNodeState,
    /// The current quorum view we know of
    current_view: QuorumViewer,
    /// Predicates that must be satisfied for a node to be allowed to join the quorum
    predicates: Vec<QuorumPredicate>,
    /// Channel to communicate with the ordering protocol
    quorum_communication: ChannelSyncTx<QuorumReconfigurationMessage>,
    /// Channel to receive responses from the quorum
    quorum_responses: ChannelSyncRx<QuorumReconfigurationResponse>,
    /// The least amount of nodes that must be in the quorum for it to be considered stable and
    /// Therefore able to initialize the ordering protocol
    min_stable_quorum: usize,
}

impl ReplicaQuorumView {
    pub fn new(
        quorum_view: QuorumViewer,
        quorum_tx: ChannelSyncTx<QuorumReconfigurationMessage>,
        quorum_response_rx: ChannelSyncRx<QuorumReconfigurationResponse>,
        predicates: Vec<QuorumPredicate>,
        min_stable_quorum: usize) -> Self {
        Self {
            current_state: JoiningReplicaState::Init,
            tbo_queue: TboQueue::init(quorum_view.sequence_number()),
            join_node_state: QuorumNodeState::Init,
            current_view: quorum_view,
            predicates,
            quorum_communication: quorum_tx,
            quorum_responses: quorum_response_rx,
            min_stable_quorum,
        }
    }

    pub fn iterate<NT>(&mut self, seq_no: &mut SeqNoGen, node_inf: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self.current_state {
            JoiningReplicaState::Init => {
                return self.broadcast_view_state_request(seq_no, node_inf, network_node, timeouts);
            }
            _ => {
                //Nothing to do here
            }
        }

        while let Ok(message) = self.quorum_responses.try_recv() {
            match message {
                QuorumReconfigurationResponse::QuorumStableResponse(success) => {
                    match self.current_state {
                        JoiningReplicaState::InitializedWaitingForOrderProtocol => {
                            if success {
                                // The ordering protocol has assimilated the quorum view, we can now
                                // Start to attempt to join it
                                debug!("Received stable quorum response from ordering protocol, starting to join quorum");

                                return self.begin_quorum_join_procedure(node_inf, network_node, timeouts);
                            } else {
                                warn!("Received stable quorum response from ordering protocol, but it failed to assimilate the quorum view. Retrying...");

                                return self.broadcast_view_state_request(seq_no, node_inf, network_node, timeouts);
                            }
                        }
                        JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol => {
                            info!("Received stable quorum response from ordering protocol, we are now stable");

                            // This happens when we are already a part of the quorum and we immediately start execution (boostrap node cases)
                            self.current_state = JoiningReplicaState::Stable;

                            return QuorumNodeResponse::Done;
                        }
                        _ => {
                            warn!("Received a stable quorum response, but we are not waiting for one {:?}. Ignoring...", self.current_state)
                        }
                    }
                }
                QuorumReconfigurationResponse::QuorumAlterationResponse(response) => {
                    match &self.join_node_state {
                        QuorumNodeState::WaitingQuorum(waiting_for) => {
                            debug!("Received a quorum alteration response from node {:?} while waiting for node {:?}.", response, waiting_for);

                            match response {
                                QuorumAlterationResponse::Successful(node) if *waiting_for == node => {
                                    if let Some(view) = self.handle_finished_quorum_entrance(node_inf, network_node, node) {
                                        return QuorumNodeResponse::NewView(view);
                                    }
                                }
                                QuorumAlterationResponse::Successful(node) => {
                                    unreachable!("Received a successful quorum alteration response for node {:?}, but we are waiting for one from node {:?}.", node, *waiting_for)
                                }
                                QuorumAlterationResponse::Failed(node, reason) => {
                                    self.handle_failed_quorum_entrance(network_node, node, reason);
                                }
                            }
                        }
                        _ => {
                            unreachable!("Received a quorum alteration response, but we are not waiting for one {:?}. Ignoring...", self.join_node_state)
                        }
                    }
                }
                QuorumReconfigurationResponse::QuorumAttemptJoinResponse(attempt_join) => {
                    match self.current_state {
                        JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol => {
                            match attempt_join {
                                QuorumAttemptJoinResponse::Success => {
                                    return self.handle_quorum_entered(node_inf);
                                }
                                QuorumAttemptJoinResponse::Failed => {}
                            }
                        }
                        _ => {
                            warn!("Received a quorum attempt join response, but we are not waiting for one {:?}. Ignoring...", self.current_state)
                        }
                    }
                }
            }
        }

        while let QuorumNodePollResult::Execute(message) = self.poll_tbo_queue() {
            let (header, message) = message.into_inner();

            self.handle_quorum_reconfig_message(node_inf, network_node, header, message);
        }

        QuorumNodeResponse::Nil
    }

    pub fn handle_view_state_message<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo,
                                         network_node: &Arc<NT>, timeouts: &Timeouts, quorum_view_state: QuorumViewCert)
                                         -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            JoiningReplicaState::Init => {
                //Remains from previous initializations?
            }
            JoiningReplicaState::Initializing(sent_messages, received, received_states) => {
                // We are still initializing

                let sender = quorum_view_state.header().from();
                let digest = quorum_view_state.header().digest().clone();

                if received.insert(sender) {
                    let entry = received_states.entry(digest)
                        .or_insert(Default::default());

                    entry.push(quorum_view_state.clone());
                } else {
                    error!("Received duplicate message from node {:?} with digest {:?}",
                           sender, digest);

                    return QuorumNodeResponse::Nil;
                }

                let needed_messages = *sent_messages / 2 + 1;

                debug!("Received {:?} messages out of {:?} needed. Latest message digest {:?}", received.len(), needed_messages, digest);

                if received.len() >= needed_messages {
                    // We have received all of the messages that we are going to receive, so we can now
                    // determine the current quorum view

                    let mut received_messages = Vec::new();

                    for (message_digests, messages) in received_states.iter() {
                        received_messages.push((message_digests.clone(), messages.clone()));
                    }

                    received_messages.sort_by(|(_, a), (_, b)| {
                        a.len().cmp(&b.len()).reverse()
                    });

                    debug!("Processed received messages: {:?}", received_messages);

                    if let Some((quorum_digest, quorum_certs)) = received_messages.first() {
                        if quorum_certs.len() >= needed_messages {
                            let view = quorum_certs.first().unwrap().message().clone();

                            let current_view = self.current_view.view();

                            self.current_view.install_view(view.clone());

                            self.installed_view_seq(view.sequence_number());

                            self.start_join_quorum(seq_no, node, network_node, timeouts);

                            return QuorumNodeResponse::NewView(view);
                        } else {
                            error!("Received {:?} messages for quorum view {:?}, but needed {:?} messages",
                                   quorum_certs.len(), quorum_digest, needed_messages);
                        }
                    } else {
                        error!("Received no messages from any nodes");
                    }
                }
            }
            _ => {
                //Nothing to do here
            }
        }

        QuorumNodeResponse::Nil
    }

    pub fn handle_view_state_request<NT>(&self, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, seq: SeqNo) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let quorum_view = self.current_view.view();

        debug!("Received view state request from node {:?} with seq {:?}, replying with {:?}", header.from(), seq, quorum_view);

        let quorum_reconfig_msg = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::NetworkViewState(quorum_view));

        network_node.send_reconfig_message(ReconfigurationMessage::new(seq, quorum_reconfig_msg), header.from());

        QuorumNodeResponse::Nil
    }

    pub fn handle_timeout<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            JoiningReplicaState::Initializing(_, _, _) => {
                self.broadcast_view_state_request(seq_no, node, network_node, timeouts)
            }
            JoiningReplicaState::JoiningQuorum(_, _, _, _) => {
                self.begin_quorum_join_procedure(node, network_node, timeouts)
            }
            _ => {
                //Nothing to do here
                QuorumNodeResponse::Nil
            }
        }
    }

    /// Broadcast the view state request to all known nodes so we can obtain the current quorum view
    fn broadcast_view_state_request<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let reconf_message = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::NetworkViewStateRequest);

        let known_nodes: Vec<_> = node.network_view.known_nodes_and_types().iter().filter_map(|(node, node_type)| {
            match node_type {
                NodeType::Replica => Some(*node),
                NodeType::Client => None,
            }
        }).collect();

        let contacted_nodes = known_nodes.len();

        let msg_seq = seq_no.next_seq();

        info!("{:?} // Broadcasting network view state request to {:?}. Seq {:?}", node.network_view.node_id(), known_nodes, msg_seq);

        let reconfig_message = ReconfigurationMessage::new(msg_seq, reconf_message);

        let _ = network_node.broadcast_reconfig_message(reconfig_message, known_nodes.into_iter());

        timeouts.timeout_reconfig_request(TIMEOUT_DUR, ((contacted_nodes * 2 / 3) + 1) as u32, msg_seq);

        self.current_state = JoiningReplicaState::Initializing(contacted_nodes, Default::default(), Default::default());

        QuorumNodeResponse::Nil
    }

    /// Start to join the quorum
    pub fn start_join_quorum<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_quorum_members = self.current_view.quorum_members();

        if current_quorum_members.is_empty() || current_quorum_members.contains(&node.network_view.node_id()) {
            warn!("We are already a part of the quorum, waiting for min stable quorum {:?} vs current {:?}", self.min_stable_quorum, current_quorum_members.len());

            self.current_state = JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol;

            if current_quorum_members.len() >= self.min_stable_quorum {

                info!("Sending stable protocol message to ordering protocol with quorum {:?}, we are part of the quorum", current_quorum_members);

                self.quorum_communication.send(QuorumReconfigurationMessage::ReconfigurationProtocolStable(current_quorum_members)).unwrap();
            }
        } else {
            match self.current_state {
                JoiningReplicaState::Initializing(_, _, _) => {
                    self.current_state = JoiningReplicaState::InitializedWaitingForOrderProtocol;

                    info!("Sending stable protocol message to ordering protocol with quorum {:?}, we are NOT part of the quorum", current_quorum_members);

                    self.quorum_communication.send(QuorumReconfigurationMessage::ReconfigurationProtocolStable(current_quorum_members.clone())).unwrap();

                    return;
                }
                _ => { /* We only have to deliver the state if it is the first time we are receive the quorum information */ }
            }

            self.begin_quorum_join_procedure(node, network_node, timeouts);
        }
    }

    fn begin_quorum_join_procedure<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let (seq, current_quorum_members) = {
            (self.current_view.sequence_number(), self.current_view.quorum_members())
        };

        info!("Starting join quorum procedure, contacting {:?}", current_quorum_members);

        self.current_state = JoiningReplicaState::JoiningQuorum(0, None, Default::default(), Default::default());

        let reconf_message = QuorumReconfigMessage::QuorumEnterRequest(QuorumEnterRequest::new(node.network_view.node_triple()));

        let reconfig_message = ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(reconf_message));

        timeouts.timeout_reconfig_request(TIMEOUT_DUR, ((current_quorum_members.len() * 2 / 3) + 1) as u32, seq);

        let _ = network_node.broadcast_reconfig_message(reconfig_message, current_quorum_members.into_iter());

        return QuorumNodeResponse::Nil;
    }

    pub fn handle_quorum_enter_response<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo,
                                            network_node: &Arc<NT>, timeouts: &Timeouts,
                                            header: Header, message: QuorumEnterResponse) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_quorum = self.current_view.quorum_members();

        return match &mut self.current_state {
            JoiningReplicaState::JoiningQuorum(accepted, view, rejected, responded) => {
                if !responded.insert(header.from()) {
                    debug!("Received duplicate response from {:?}", header.from());

                    return QuorumNodeResponse::Nil;
                }

                match message {
                    QuorumEnterResponse::Successful(rcvd_view) if view.is_some() && *view.as_ref().unwrap() != rcvd_view => {
                        warn!("Received successful response, but the view is different from the one we have already received");
                    }
                    QuorumEnterResponse::Successful(rcvd_view) => {
                        *accepted += 1;

                        if let None = view {
                            *view = Some(rcvd_view);
                        }
                    }
                    QuorumEnterResponse::Rejected(rejection) => {
                        let count = rejected.entry(rejection).or_insert(0);

                        *count += 1;
                    }
                }

                let reject_count: usize = rejected.values().sum();

                if *accepted >= necessary_votes(&current_quorum) {
                    let view = view.as_ref().unwrap().clone();

                    // The node has been accepted into the quorum
                    self.handle_accepted_into_quorum(node, view);
                } else if reject_count >= necessary_votes(&current_quorum) {
                    // We have been rejected, retry and hope for the best
                    self.broadcast_view_state_request(seq_no, node, network_node, timeouts);
                } else if responded.len() >= current_quorum.len() {
                    // Well nothing seems to have happened, retry
                    self.begin_quorum_join_procedure(node, network_node, timeouts);
                }

                QuorumNodeResponse::Nil
            }
            _ => {
                debug!("Received Quorum Enter Response while not in joining quorum state, ignoring");
                QuorumNodeResponse::Nil
            }
        };
    }

    /// Handle us having been accepted into the quorum
    fn handle_accepted_into_quorum(&mut self, node: &GeneralNodeInfo, view: QuorumView) -> QuorumNodeResponse {
        info!("We have been accepted into the quorum, sending confirm join message, installing view {:?} and sending AttemptToJoinQuorum message to order protocol.", view);

        self.current_state = JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol;

        let prev_seq = self.current_view.sequence_number();

        self.current_view.install_view(view.clone());

        self.installed_view_seq(view.sequence_number());

        self.quorum_communication.send(QuorumReconfigurationMessage::AttemptToJoinQuorum).unwrap();

        return QuorumNodeResponse::NewView(view);
    }

    /// Handle a given node having entered
    pub fn handle_quorum_entered(&mut self, node: &GeneralNodeInfo) -> QuorumNodeResponse {
        self.current_state = JoiningReplicaState::Stable;

        return QuorumNodeResponse::Done;
    }

    pub fn handle_quorum_reconfig_message<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: ReconfigurationMessage) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_seq = self.tbo_queue.sequence_number();
        let current_quorum = self.current_view.quorum_members();

        match &mut self.join_node_state {
            QuorumNodeState::Init => {
                debug!("Received a quorum reconfiguration message while in the init state. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                self.tbo_queue.queue(header, message);
            }
            QuorumNodeState::Joining(votes, voted) | QuorumNodeState::JoiningVoted(votes, voted) => {
                let received = match reconfig_quorum_message_type(&message) {
                    ReconfigQuorumMessage::JoinMessage(_) if message.sequence_number() != current_seq => {
                        debug!("Received a quorum join message while in join phase, but with wrong sequence number. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(header, message);

                        return QuorumNodeResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(_) => {
                        debug!("Received a quorum confirm join message while in the joining state. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(header, message);

                        return QuorumNodeResponse::Nil;
                    }
                    ReconfigQuorumMessage::JoinMessage(node) => {
                        *node
                    }
                };

                if !voted.insert(header.from()) {
                    debug!("Received a quorum join message from node {:?}, but he has already voted for this round? Ignoring it", header.from());

                    return QuorumNodeResponse::Nil;
                }

                let votes_for_received = votes.entry(received).or_insert(0);

                *votes_for_received += 1;

                let needed_votes = necessary_votes(&current_quorum);

                if *votes_for_received >= needed_votes {
                    debug!("Received enough votes to join the quorum. Sending a confirm join message");

                    let confirm_message = ReconfigQuorumMessage::ConfirmJoin(received);

                    let reconfig_message = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumReconfig(confirm_message));

                    let message = ReconfigurationMessage::new(current_seq, reconfig_message);

                    network_node.broadcast_reconfig_message(message, current_quorum.into_iter());

                    self.join_node_state = QuorumNodeState::ConfirmingJoin(received, Default::default());
                } else if *votes_for_received >= current_quorum.len() {
                    votes.iter().for_each(|(node, votes)| {
                        let rejected_msg = QuorumEnterResponse::Rejected(QuorumEnterRejectionReason::FailedToObtainQuorum);
                    });
                }
            }
            QuorumNodeState::ConfirmingJoin(received, voted) => {
                match reconfig_quorum_message_type(&message) {
                    ReconfigQuorumMessage::JoinMessage(_) => {
                        debug!("Received a quorum join message while in the confirming join state. Queuing it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(header, message);

                        return QuorumNodeResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(node) if message.sequence_number() != current_seq => {
                        debug!("Received a quorum confirm join message while in the confirming join state, but with wrong sequence number. Queuing it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(header, message);

                        return QuorumNodeResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(node) if node != received => {
                        debug!("Received a quorum confirm join message from node {:?}, but we are waiting for {:?}. Ignoring it", node, received);

                        return QuorumNodeResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(_) => {}
                };

                if !voted.insert(header.from()) {
                    debug!("Received a quorum confirm join message from node {:?}, but he has already voted for this round? Ignoring it", header.from());

                    return QuorumNodeResponse::Nil;
                }

                debug!("Received a quorum confirm join message from node {:?} for node {:?}.", header.from(), received);

                if voted.len() >= necessary_votes(&current_quorum) {
                    info!("Received enough confirm join messages for node {:?} to join the quorum. Sending update to order protocol and waiting for response", received);

                    let entered_node = *received;

                    self.join_node_state = QuorumNodeState::WaitingQuorum(entered_node);

                    self.request_entrance_from_quorum(entered_node);
                }
            }
            QuorumNodeState::WaitingQuorum(_) => {
                return match reconfig_quorum_message_type(&message) {
                    ReconfigQuorumMessage::JoinMessage(_) => {
                        debug!("Received a quorum join message while in the waiting quorum state. Queuing it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(header, message);

                        QuorumNodeResponse::Nil
                    }
                    ReconfigQuorumMessage::ConfirmJoin(_) => {
                        debug!("Received a quorum confirm join message while in the waiting quorum state. Queuing it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(header, message);

                        QuorumNodeResponse::Nil
                    }
                };
            }
        }

        QuorumNodeResponse::Nil
    }

    fn broadcast_quorum_updated<NT>(&self, node: &GeneralNodeInfo, network_node: &Arc<NT>)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_view = self.current_view.view();

        let seq = current_view.sequence_number();

        let message = QuorumReconfigMessage::QuorumUpdate(current_view);

        let reconfig_message = ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(message));

        let known_nodes = node.network_view.known_nodes();

        network_node.broadcast_reconfig_message(reconfig_message, known_nodes.into_iter());
    }

    fn handle_finished_quorum_entrance<NT>(&mut self, node_inf: &GeneralNodeInfo, network_node: &Arc<NT>, node: NodeId) -> Option<QuorumView>
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self.join_node_state {
            QuorumNodeState::WaitingQuorum(waiting_node)  if waiting_node == node => {
                let (prev, curr) = {
                    let current_view = self.current_view.view();

                    let new_view = current_view.next_with_added_node(node);

                    self.current_view.install_view(new_view.clone());

                    (current_view, new_view)
                };

                self.join_node_state = QuorumNodeState::Init;

                debug!("Node {:?} has been added to the quorum, broadcasting QuorumEnterResponse", node);

                let response = QuorumEnterResponse::Successful(curr.clone());

                let message = QuorumReconfigMessage::QuorumEnterResponse(response);

                let reconf_message = ReconfigurationMessage::new(prev.sequence_number(), ReconfigurationMessageType::QuorumReconfig(message));

                network_node.send_reconfig_message(reconf_message, node);

                self.broadcast_quorum_updated(node_inf, network_node);

                self.installed_view_seq(curr.sequence_number());

                return Some(curr);
            }
            QuorumNodeState::WaitingQuorum(waiting) => {
                error!("Received a finished quorum entrance message for node {:?}, but we are waiting for node {:?}. Ignoring it", node, waiting);
            }
            _ => {
                error!("Attempt to finish quorum entrance, but we are not waiting for one. Ignoring it");
            }
        }

        None
    }

    fn handle_failed_quorum_entrance<NT>(&mut self, network_node: &Arc<NT>, node: NodeId, reason: AlterationFailReason)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        self.join_node_state = QuorumNodeState::Init;

        let current_quorum_seq = self.current_view.sequence_number();

        let response = match reason {
            AlterationFailReason::Failed => {
                QuorumEnterResponse::Rejected(QuorumEnterRejectionReason::NotAuthorized)
            }
            AlterationFailReason::OngoingReconfiguration => {
                QuorumEnterResponse::Rejected(QuorumEnterRejectionReason::CurrentlyReconfiguring)
            }
            AlterationFailReason::AlreadyPartOfQuorum => {
                QuorumEnterResponse::Rejected(QuorumEnterRejectionReason::AlreadyPartOfQuorum)
            }
        };

        let message = QuorumReconfigMessage::QuorumEnterResponse(response);

        let reconf_message = ReconfigurationMessage::new(current_quorum_seq, ReconfigurationMessageType::QuorumReconfig(message));

        network_node.send_reconfig_message(reconf_message, node);
    }

    fn request_entrance_from_quorum(&mut self, node: NodeId) {
        self.quorum_communication.send(QuorumReconfigurationMessage::RequestQuorumJoin(node)).unwrap();
    }

    pub fn handle_quorum_enter_request<NT>(&mut self, seq: SeqNo, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: QuorumEnterRequest) -> QuorumNodeResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_view = self.current_view.view();

        let node_triple = message.into_inner();

        let our_node_id = node.network_view.node_id();

        return match self.current_state {
            JoiningReplicaState::Stable => {
                let network_node = network_node.clone();

                let predicates = self.predicates.clone();

                match &self.join_node_state {
                    QuorumNodeState::Init | QuorumNodeState::Joining(_, _) => {}
                    _ => {
                        let result = QuorumEnterResponse::Rejected(QuorumEnterRejectionReason::CurrentlyReconfiguring);

                        let quorum_reconfig_msg = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(result));

                        let reconf_message = ReconfigurationMessage::new(seq, quorum_reconfig_msg);

                        let _ = network_node.send_reconfig_message(reconf_message, header.from());

                        return QuorumNodeResponse::Nil;
                    }
                }

                if current_view.sequence_number() != seq {
                    // We have received a request for the wrong quorum view sequence number

                    let result = QuorumEnterResponse::Rejected(QuorumEnterRejectionReason::IncorrectNetworkViewSeq);

                    let quorum_reconfig_msg = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(result));

                    let reconf_message = ReconfigurationMessage::new(seq, quorum_reconfig_msg);

                    let _ = network_node.send_reconfig_message(reconf_message, header.from());

                    return QuorumNodeResponse::Nil;
                }

                let mut responses = Vec::new();

                for predicate in predicates {
                    responses.push(predicate(current_view.clone(), node_triple.clone()));
                }

                let responses = rt::block_on(join_all(responses));

                for join_result in responses {
                    if let Some(reason) = join_result.unwrap() {
                        let result = QuorumEnterResponse::Rejected(reason);

                        let quorum_reconfig_msg = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(result));

                        let reconf_message = ReconfigurationMessage::new(seq, quorum_reconfig_msg);

                        let _ = network_node.send_reconfig_message(reconf_message, header.from());

                        return QuorumNodeResponse::Nil;
                    }
                }

                debug!("Node {:?} has been approved to join the quorum", node_triple.node_id());

                self.begin_quorum_node_accept_procedure(node, &network_node, node_triple);

                QuorumNodeResponse::Nil
            }
            _ => {
                warn!("Received a quorum enter request while not in stable state, ignoring. Current state: {:?}", self.current_state);

                QuorumNodeResponse::Nil
            }
        };
    }

    /// Initiate the procedure to accept a node into the quorum.
    fn begin_quorum_node_accept_procedure<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, joining_node: NodeTriple)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let quorum_view = self.current_view.view();

        let mem = std::mem::replace(&mut self.join_node_state, QuorumNodeState::Init);

        self.join_node_state = match mem {
            QuorumNodeState::Joining(votes, voted) => {
                QuorumNodeState::JoiningVoted(votes, voted)
            }
            _ => {
                QuorumNodeState::JoiningVoted(Default::default(), Default::default())
            }
        };

        let seq = quorum_view.sequence_number();

        let message = ReconfigQuorumMessage::JoinMessage(joining_node.node_id());

        let reconf_message = ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumReconfig(message)));

        network_node.broadcast_reconfig_message(reconf_message, quorum_view.quorum_members().clone().into_iter());
    }

    fn poll_tbo_queue(&mut self) -> QuorumNodePollResult {
        match self.join_node_state {
            QuorumNodeState::Init => {
                if self.tbo_queue.has_pending_join_message() {
                    debug!("We have a pending join message to process, moving to joining phase and processing the messages");

                    self.join_node_state = QuorumNodeState::Joining(Default::default(), Default::default());

                    return QuorumNodePollResult::Execute(self.tbo_queue.pop_join_message().unwrap());
                }
            }
            QuorumNodeState::Joining(_, _) | QuorumNodeState::JoiningVoted(_, _) => {
                if self.tbo_queue.has_pending_join_message() {
                    return QuorumNodePollResult::Execute(self.tbo_queue.pop_join_message().unwrap());
                }
            }
            QuorumNodeState::ConfirmingJoin(_, _) => {
                if self.tbo_queue.has_pending_confirm_messages() {
                    return QuorumNodePollResult::Execute(self.tbo_queue.pop_confirm_message().unwrap());
                }
            }
            QuorumNodeState::WaitingQuorum(_) => {}
        }

        QuorumNodePollResult::None
    }

    fn installed_view_seq(&mut self, seq: SeqNo) {
        match seq.index(self.tbo_queue.curr_seq) {
            Either::Left(_) => {}
            Either::Right(adv) => {
                for _ in 0..adv {
                    self.tbo_queue.advance();
                }
            }
        }
    }
}

impl TboQueue {
    fn init(seq: SeqNo) -> Self {
        Self {
            curr_seq: seq,
            joining_message: Default::default(),
            confirm_join_message: Default::default(),
        }
    }

    fn queue(&mut self, header: Header, message: ReconfigurationMessage) {
        match reconfig_quorum_message_type(&message) {
            ReconfigQuorumMessage::JoinMessage(_) => {
                tbo_queue_message(self.curr_seq, &mut self.joining_message, StoredMessage::new(header, message))
            }
            ReconfigQuorumMessage::ConfirmJoin(_) => {
                tbo_queue_message(self.curr_seq, &mut self.confirm_join_message, StoredMessage::new(header, message))
            }
        }
    }

    fn has_pending_join_message(&self) -> bool {
        if let Some(first) = self.joining_message.front() {
            return !first.is_empty();
        }

        false
    }

    fn has_pending_confirm_messages(&self) -> bool {
        if let Some(first) = self.confirm_join_message.front() {
            return !first.is_empty();
        }

        false
    }

    fn pop_join_message(&mut self) -> Option<StoredMessage<ReconfigurationMessage>> {
        tbo_pop_message(&mut self.joining_message)
    }

    fn pop_confirm_message(&mut self) -> Option<StoredMessage<ReconfigurationMessage>> {
        tbo_pop_message(&mut self.confirm_join_message)
    }

    fn advance(&mut self) {
        self.curr_seq = self.curr_seq.next();

        tbo_advance_message_queue(&mut self.joining_message);
        tbo_advance_message_queue(&mut self.confirm_join_message);
    }
}

impl Orderable for TboQueue {
    fn sequence_number(&self) -> SeqNo {
        self.curr_seq
    }
}


fn reconfig_quorum_message_type(message: &ReconfigurationMessage) -> &ReconfigQuorumMessage {
    match message.message_type() {
        ReconfigurationMessageType::QuorumReconfig(message_type) => {
            match message_type {
                QuorumReconfigMessage::QuorumReconfig(message_type) => {
                    message_type
                }
                _ => unreachable!()
            }
        }
        _ => unreachable!(),
    }
}

