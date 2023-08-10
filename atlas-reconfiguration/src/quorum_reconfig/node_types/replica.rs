use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, RwLock};
use futures::future::join_all;

use log::{debug, error, info, warn};

use atlas_common::async_runtime as rt;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo, tbo_pop_message, tbo_queue_message};
use atlas_common::threadpool::join;
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use atlas_core::reconfiguration_protocol::{QuorumAlterationResponse, QuorumReconfigurationMessage, QuorumReconfigurationResponse, QuorumUpdateMessage};
use atlas_core::timeouts::Timeouts;

use crate::{GeneralNodeInfo, QuorumProtocolResponse, SeqNoGen, TIMEOUT_DUR};
use crate::message::{NodeTriple, QuorumEnterRejectionReason, QuorumEnterRequest, QuorumEnterResponse, QuorumJoinCertificate, QuorumNodeJoinApproval, QuorumReconfigMessage, QuorumViewCert, ReconfData, ReconfigQuorumMessage, ReconfigurationMessage, ReconfigurationMessageType};
use crate::quorum_reconfig::{QuorumPredicate, QuorumView};

struct TboQueue {
    signal: bool,

    joining_message: VecDeque<VecDeque<StoredMessage<ReconfigurationMessage>>>,
    confirm_join_message: VecDeque<VecDeque<StoredMessage<ReconfigurationMessage>>>,
}

/// The state of a replica
enum JoiningReplicaState {
    Init,
    /// We are initializing our state so we know who to contact in order to join the quorum
    Initializing(usize, BTreeSet<NodeId>, BTreeMap<Digest, Vec<QuorumViewCert>>),
    /// We have initialized our state and are waiting for the response of the ordering protocol
    InitializedWaitingForOrderProtocol,
    /// We are currently joining the quorum
    JoiningQuorum(usize, BTreeSet<NodeId>),
    /// We have been approved by the reconfiguration protocol and we are now waiting
    /// for the order protocol to respond
    JoinedQuorumWaitingForOrderProtocol,
    /// We are currently part of the quorum and are stable
    Stable,
}

/// The current state of the ordering protocol
/// Depending on the state, we will handle the messages we send to the ordering protocol
/// If the order protocol is still waiting for enough members to join, then we will
/// deliver a stable message to the ordering protocol once we have enough members.
/// If the order protocol is already running, then we will deliver a stable message
/// with the current members, followed by a quorum join message to authorize our entry
enum OrderProtocolState {
    Waiting,
    Running,
}

/// The node state of a quorum node, waiting for new nodes to join
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
    WaitingQuorum,
}

/// The replica's quorum view and all of the necessary states
pub(crate) struct ReplicaQuorumView {
    /// The current state of the replica
    current_state: JoiningReplicaState,
    /// The current state of the ordering protocol
    order_protocol_state: OrderProtocolState,
    tbo_queue: TboQueue,
    /// The current state of nodes joining
    join_node_state: QuorumNodeState,
    /// The current quorum view we know of
    current_view: Arc<RwLock<QuorumView>>,
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
        quorum_view: Arc<RwLock<QuorumView>>,
        quorum_tx: ChannelSyncTx<QuorumReconfigurationMessage>,
        quorum_response_rx: ChannelSyncRx<QuorumReconfigurationResponse>,
        predicates: Vec<QuorumPredicate>,
        min_stable_quorum: usize) -> Self {
        Self {
            current_state: JoiningReplicaState::Init,
            order_protocol_state: OrderProtocolState::Waiting,
            tbo_queue: TboQueue::init(),
            join_node_state: QuorumNodeState::Init,
            current_view: quorum_view,
            predicates,
            quorum_communication: quorum_tx,
            quorum_responses: quorum_response_rx,
            min_stable_quorum,
        }
    }

    pub fn iterate<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self.current_state {
            JoiningReplicaState::Init => {
                let reconf_message = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::NetworkViewStateRequest);

                let known_nodes = node.network_view.known_nodes();

                let contacted_nodes = known_nodes.len();

                info!("{:?} // Broadcasting network view state request to {:?}", node.network_view.node_id(), known_nodes);

                let reconfig_message = ReconfigurationMessage::new(seq_no.next_seq(), reconf_message);

                let _ = network_node.broadcast_reconfig_message(reconfig_message, known_nodes.into_iter());

                timeouts.timeout_reconfig_request(TIMEOUT_DUR, ((contacted_nodes * 2 / 3) + 1) as u32, seq_no.curr_seq());

                self.current_state = JoiningReplicaState::Initializing(contacted_nodes, Default::default(), Default::default());

                return QuorumProtocolResponse::Running;
            }
            _ => {
                //Nothing to do here
            }
        }

        while let Ok(message) = self.quorum_responses.try_recv() {
            match message {
                QuorumReconfigurationResponse::QuorumAlterationResponse(response) => {
                    match response {
                        QuorumAlterationResponse::Successful => {
                            match self.current_state {
                                JoiningReplicaState::InitializedWaitingForOrderProtocol => {
                                    return self.broadcast_join_message(seq_no, node, network_node, timeouts);
                                }
                                JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol => {
                                    return self.handle_quorum_entered(node);
                                }
                                _ => {
                                    warn!("Received a successful quorum alteration response, but we are not waiting for one. Ignoring...")
                                }
                            }
                        }
                        QuorumAlterationResponse::Failed(reason) => {
                            match self.current_state {
                                JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol => {
                                    return self.start_join_quorum(seq_no, node, network_node, timeouts);
                                }
                                JoiningReplicaState::InitializedWaitingForOrderProtocol => {
                                    panic!("We have failed to deliver the quorum information to the ordering protocol. Aborting program...")
                                }
                                _ => {
                                    warn!("Received a failed quorum alteration response, but we are not waiting for one. Ignoring...")
                                }
                            }
                        }
                    }
                }
                QuorumReconfigurationResponse::QuorumUpdate(update) => {
                    match update {
                        QuorumUpdateMessage::UpdatedQuorumView(updated_quorum_view) => {
                            let mut quorum_view = self.current_view.write().unwrap();

                            let quorum_nodes = quorum_view.quorum_members().clone();

                            let added_node = updated_quorum_view.into_iter().find(|node| !quorum_nodes.contains(node));

                            if let Some(added_node) = added_node {
                                let view = quorum_view.next_with_added_node(added_node);

                                *quorum_view = view;

                                info!("Received a quorum update, adding node {:?}, new view now contains {:?}", added_node, quorum_view.quorum_members());
                            } else {
                                warn!("Received a quorum update, but no new node was added. Ignoring...");
                            }
                        }
                    }
                }
            }
        }

        QuorumProtocolResponse::Nil
    }

    pub fn handle_view_state_message<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo,
                                         network_node: &Arc<NT>, timeouts: &Timeouts, quorum_view_state: QuorumViewCert)
                                         -> QuorumProtocolResponse
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

                    return QuorumProtocolResponse::Running;
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
                            {
                                let mut write_guard = self.current_view.write().unwrap();

                                *write_guard = quorum_certs.first().unwrap().message().clone();
                            }

                            return self.start_join_quorum(seq_no, node, network_node, timeouts);
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

        QuorumProtocolResponse::Running
    }

    pub fn handle_view_state_request<NT>(&self, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, seq: SeqNo) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let quorum_view = self.current_view.read().unwrap().clone();

        debug!("Received view state request from node {:?} with seq {:?}, replying with {:?}", header.from(), seq, quorum_view);

        let quorum_reconfig_msg = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::NetworkViewState(quorum_view));

        network_node.send_reconfig_message(ReconfigurationMessage::new(seq, quorum_reconfig_msg), header.from());

        QuorumProtocolResponse::Nil
    }

    pub fn handle_timeout<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            JoiningReplicaState::Initializing(_, _, _) => {
                let reconf_message = QuorumReconfigMessage::NetworkViewStateRequest;

                let known_nodes = node.network_view.known_nodes();

                let contacted_nodes = known_nodes.len();

                let reconfig_message = ReconfigurationMessage::new(seq_no.next_seq(), ReconfigurationMessageType::QuorumReconfig(reconf_message));

                let _ = network_node.broadcast_reconfig_message(reconfig_message, known_nodes.into_iter());

                self.current_state = JoiningReplicaState::Initializing(contacted_nodes, Default::default(), Default::default());

                QuorumProtocolResponse::Running
            }
            JoiningReplicaState::JoiningQuorum(_, _) => {
                let enter_request = QuorumEnterRequest::new(node.network_view.node_triple());

                let quorum_enter_request = QuorumReconfigMessage::QuorumEnterRequest(enter_request);

                let reconfig_message = ReconfigurationMessage::new(seq_no.next_seq(), ReconfigurationMessageType::QuorumReconfig(quorum_enter_request));

                let quorum_members = self.current_view.read().unwrap().quorum_members().clone();

                let contacted_nodes = quorum_members.len();

                let _ = network_node.broadcast_reconfig_message(reconfig_message, quorum_members.into_iter());

                self.current_state = JoiningReplicaState::JoiningQuorum(contacted_nodes, Default::default());

                QuorumProtocolResponse::Running
            }
            _ => {
                //Nothing to do here
                QuorumProtocolResponse::Nil
            }
        }
    }

    pub fn start_join_quorum<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_quorum_members = self.current_view.read().unwrap().quorum_members().clone();

        if current_quorum_members.is_empty() || current_quorum_members.contains(&node.network_view.node_id()) {
            warn!("We are already a part of the quorum, moving to stable");

            self.current_state = JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol;

            if current_quorum_members.len() >= self.min_stable_quorum {
                self.quorum_communication.send(QuorumReconfigurationMessage::ReconfigurationProtocolStable(current_quorum_members)).unwrap();

                QuorumProtocolResponse::Running
            } else {
                QuorumProtocolResponse::Running
            }
        } else {
            match self.current_state {
                JoiningReplicaState::Initializing(_, _, _) => {
                    self.current_state = JoiningReplicaState::InitializedWaitingForOrderProtocol;

                    self.quorum_communication.send(QuorumReconfigurationMessage::ReconfigurationProtocolStable(current_quorum_members.clone())).unwrap();

                    return QuorumProtocolResponse::Running;
                }
                _ => { /* We only have to deliver the state if it is the first time we are receive the quorum information */ }
            }

            self.broadcast_join_message(seq_no, node, network_node, timeouts)
        }
    }

    fn broadcast_join_message<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_quorum_members = self.current_view.read().unwrap().quorum_members().clone();

        info!("Starting join quorum procedure, contacting {:?}", current_quorum_members);

        self.current_state = JoiningReplicaState::JoiningQuorum(current_quorum_members.len(), Default::default());

        let reconf_message = QuorumReconfigMessage::QuorumEnterRequest(QuorumEnterRequest::new(node.network_view.node_triple()));

        let reconfig_message = ReconfigurationMessage::new(seq_no.next_seq(), ReconfigurationMessageType::QuorumReconfig(reconf_message));

        timeouts.timeout_reconfig_request(TIMEOUT_DUR, ((current_quorum_members.len() * 2 / 3) + 1) as u32, seq_no.curr_seq());

        let _ = network_node.broadcast_reconfig_message(reconfig_message, current_quorum_members.into_iter());

        return QuorumProtocolResponse::Running;
    }

    /// Handle us having received a request to join the quorum
    pub fn handle_quorum_enter_request<NT>(&mut self, seq: SeqNo, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: QuorumEnterRequest) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let quorum_members = self.current_view.read().unwrap().quorum_members().clone();

        let node_triple = message.into_inner();

        let our_node_id = node.network_view.node_id();

        match self.current_state {
            JoiningReplicaState::Stable | JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol => {
                if quorum_members.contains(&our_node_id) {
                    // We are already a part of the quorum, so we can just approve the request

                    let network_node = network_node.clone();

                    let quorum_view = self.current_view.read().unwrap().clone();

                    let predicates = self.predicates.clone();

                    rt::spawn(async move {
                        let mut responses = Vec::new();

                        for predicate in predicates {
                            responses.push(predicate(quorum_view.clone(), node_triple.clone()));
                        }

                        let responses = join_all(responses).await;

                        for join_result in responses {
                            if let Some(reason) = join_result.unwrap() {
                                let result = QuorumEnterResponse::Rejected(reason);

                                let quorum_reconfig_msg = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(result));

                                let reconf_message = ReconfigurationMessage::new(seq, quorum_reconfig_msg);

                                let _ = network_node.send_reconfig_message(reconf_message, header.from());

                                return;
                            }
                        }

                        debug!("Node {:?} has been approved to join the quorum, replying with node join approval", node_triple.node_id());

                        // If all predicates pass, then he should be in the clear
                        let join_approval =
                            QuorumNodeJoinApproval::new(quorum_view.sequence_number(),
                                                        node_triple.node_id(),
                                                        our_node_id);

                        let join_approval = QuorumEnterResponse::Successful(join_approval);

                        network_node.send_reconfig_message(ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(join_approval))), header.from());
                    });

                    QuorumProtocolResponse::Nil
                } else {
                    let enter_response = QuorumReconfigMessage::QuorumEnterResponse(QuorumEnterResponse::Rejected(QuorumEnterRejectionReason::NodeIsNotQuorumParticipant));

                    network_node.send_reconfig_message(ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(enter_response)), header.from());

                    QuorumProtocolResponse::Nil
                }
            }
            _ => {
                QuorumProtocolResponse::Nil
            }
        }
    }

    pub fn handle_quorum_enter_response<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts, header: Header, message: QuorumEnterResponse) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            JoiningReplicaState::JoiningQuorum(contacted, received) => {
                if received.insert(header.from()) {
                    match message {
                        QuorumEnterResponse::Successful(approval) => {
                            let join_approvals = join_approvals.entry(approval.sequence_number()).or_insert(Vec::new());

                            join_approvals.push(StoredMessage::new(header, approval));
                        }
                        QuorumEnterResponse::Rejected(rejection_reason) => {
                            error!("Received rejection from node {:?} with reason {:?}", header.from(), rejection_reason);
                        }
                    }
                }

                let necessary_response = (*contacted * 2 / 3) + 1;

                debug!("Received Quorum Enter Response. Joining Quorum State: {:?} responses out of {:?} needed", received.len(), necessary_response);

                if received.len() >= necessary_response {
                    let mut collected_join_approvals = Vec::with_capacity(join_approvals.len());

                    for (seq, approvals) in join_approvals.iter() {
                        collected_join_approvals.push((*seq, approvals.clone()));
                    }

                    collected_join_approvals.sort_by(|(seq, votes), (seq2, votes2)| {
                        return votes.len().cmp(&votes2.len()).reverse();
                    });

                    if collected_join_approvals.is_empty() {
                        todo!("Handle this case. Do we want to force a retry?");
                    }

                    debug!("Collected join approvals: {:?}", collected_join_approvals);

                    let (seq, approvals) = collected_join_approvals.swap_remove(0);

                    if approvals.len() < necessary_response {
                        return QuorumProtocolResponse::Running;
                    }

                    let certificate = QuorumJoinCertificate::new(seq, approvals.clone());

                    self.quorum_communication.send(QuorumReconfigurationMessage::AttemptToJoinQuorum).unwrap();

                    self.request_quorum_join(seq_gen, node, network_node, certificate);
                }

                QuorumProtocolResponse::Running
            }
            _ => {
                // We are not currently joining the quorum, so we can ignore this message
                QuorumProtocolResponse::Nil
            }
        }
    }

    /// Handle us having correctly entered the quorum
    pub fn handle_quorum_entered(&mut self, node: &GeneralNodeInfo) -> QuorumProtocolResponse {
        self.current_state = JoiningReplicaState::Stable;

        let mut view = self.current_view.write().unwrap();

        if !view.quorum_members.contains(&node.network_view.node_id()) {
            let new_view = view.next_with_added_node(node.network_view.node_id());

            *view = new_view;
        }

        return QuorumProtocolResponse::Done;
    }

    /// Handle the quorum view node
    pub fn handle_quorum_view_node_added<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, node_added: NodeId)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let mut guard = self.current_view.write().unwrap();

        let novel_view = guard.next_with_added_node(node_added);

        let _ = std::mem::replace(&mut *guard, novel_view);

        let reconf_message = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumUpdated((*guard).clone()));

        let known_nodes = node.network_view.known_nodes().into_iter().filter(|node| !guard.quorum_members.contains(node));

        // Only send this message to nodes that do not partake in the quorum, as those will be notified of a change by their own ordering protocols
        network_node.broadcast_reconfig_message(ReconfigurationMessage::new(seq_gen.next_seq(), reconf_message), known_nodes);
    }

    pub fn handle_request_quorum_join(&mut self, seq_gen: &SeqNoGen, node: &GeneralNodeInfo, header: Header, node_id: NodeId, jc: QuorumJoinCertificate) -> QuorumProtocolResponse {
        ///TODO: Verify validity of the join certificate before accepting

        info!("{:?} // Node {:?} has requested to join the quorum", node.network_view.node_id(), node_id);

        if jc.quorum_view_seq() != self.current_view.read().unwrap().sequence_number() {
            error!("Received a join certificate for a different view than the current one. Rejecting the request");

            return QuorumProtocolResponse::Nil;
        }

        self.quorum_communication.send(QuorumReconfigurationMessage::RequestQuorumJoin(node_id)).unwrap();

        QuorumProtocolResponse::Running
    }

    /// Request that we join the quorum
    fn request_quorum_join<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, jc: QuorumJoinCertificate)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        info!("{:?} // Requesting to join the quorum", node.network_view.node_id());

        self.current_state = JoiningReplicaState::JoinedQuorumWaitingForOrderProtocol;

        let quorum_view = self.current_view.read().unwrap().clone();

        let join_message = QuorumReconfigMessage::QuorumJoin(node.network_view.node_id(), jc);

        let message = ReconfigurationMessage::new(seq_gen.next_seq(), ReconfigurationMessageType::QuorumReconfig(join_message));

        network_node.broadcast_reconfig_message(message, quorum_view.quorum_members.clone().into_iter().filter(|node_id| *node_id != node.network_view.node_id()));
    }

    pub fn handle_quorum_reconfig_message<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: ReconfigurationMessage) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_seq = self.current_view.read().unwrap().sequence_number;
        let current_quorum = self.current_view.read().unwrap().quorum_members().clone();

        match &mut self.join_node_state {
            QuorumNodeState::Init => {
                debug!("Received a quorum reconfiguration message while in the init state. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                self.tbo_queue.queue(current_seq, header, message);
            }
            QuorumNodeState::Joining(votes, voted) | QuorumNodeState::JoiningVoted(votes, voted) => {
                let received = match reconfig_quorum_message_type(&message) {
                    ReconfigQuorumMessage::JoinMessage(_) if message.sequence_number() != current_seq => {
                        debug!("Received a quorum join message while in join phase, but with wrong sequence number. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(current_seq, header, message);

                        return QuorumProtocolResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(_) => {
                        debug!("Received a quorum confirm join message while in the joining state. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(current_seq, header, message);

                        return QuorumProtocolResponse::Nil;
                    }
                    ReconfigQuorumMessage::JoinMessage(node) => {
                        *node
                    }
                };

                if !voted.insert(header.from()) {
                    debug!("Received a quorum join message from node {:?}, but he has already voted for this round? Ignoring it", header.from());

                    return QuorumProtocolResponse::Nil;
                }

                let votes = votes.entry(received).or_insert(0);

                *votes += 1;

                let vote = necessary_votes(self.current_view.read().unwrap().quorum_members());

                if vote >= *votes {
                    debug!("Received enough votes to join the quorum. Sending a confirm join message");

                    let confirm_message = ReconfigQuorumMessage::ConfirmJoin(received);

                    let reconfig_message = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumReconfig(confirm_message));

                    let message = ReconfigurationMessage::new(current_seq, reconfig_message);

                    network_node.broadcast_reconfig_message(message, self.current_view.read().unwrap().quorum_members().clone());

                    self.join_node_state = QuorumNodeState::ConfirmingJoin(received, Default::default());
                }
            }
            QuorumNodeState::ConfirmingJoin(received, voted) => {
                match reconfig_quorum_message_type(&message) {
                    ReconfigQuorumMessage::JoinMessage(_) => {
                        debug!("Received a quorum join message while in the confirming join state. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(current_seq, header, message);

                        return QuorumProtocolResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(node) if message.sequence_number() != current_seq => {
                        debug!("Received a quorum confirm join message while in the confirming join state, but with wrong sequence number. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(current_seq, header, message);

                        return QuorumProtocolResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(node) if node != received => {
                        debug!("Received a quorum confirm join message from node {:?}, but we are waiting for {:?}. Ignoring it", node, received);

                        return QuorumProtocolResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(_) => {}
                };

                if !voted.insert(header.from()) {
                    debug!("Received a quorum confirm join message from node {:?}, but he has already voted for this round? Ignoring it", header.from());

                    return QuorumProtocolResponse::Nil;
                }

                debug!("Received a quorum confirm join message from node {:?} for node {:?}.", header.from(), received);

                if voted.len() >= necessary_votes(&current_quorum) {
                    info!("Received enough confirm join messages for node {:?} to join the quorum. Sending update to order protocol and waiting for response", received);

                    self.join_node_state = QuorumNodeState::WaitingQuorum;

                    self.request_entrance_from_quorum(*received);
                }
            }
            QuorumNodeState::WaitingQuorum => {
                match reconfig_quorum_message_type(&message) {
                    ReconfigQuorumMessage::JoinMessage(_) => {
                        debug!("Received a quorum join message while in the waiting quorum state. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(current_seq, header, message);

                        return QuorumProtocolResponse::Nil;
                    }
                    ReconfigQuorumMessage::ConfirmJoin(_) => {
                        debug!("Received a quorum confirm join message while in the waiting quorum state. Queuing it it. Seq {:?} vs Our {:?}", message.sequence_number(), current_seq);

                        self.tbo_queue.queue(current_seq, header, message);

                        return QuorumProtocolResponse::Nil;
                    }
                }
            }
        }

        QuorumProtocolResponse::Nil
    }

    fn request_entrance_from_quorum(&mut self, node: NodeId) {
        self.quorum_communication.send(QuorumReconfigurationMessage::RequestQuorumJoin(node)).unwrap();
    }

    pub fn handle_quorum_enter_request_v2<NT>(&mut self, seq: SeqNo, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: QuorumEnterRequest) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let quorum_members = self.current_view.read().unwrap().quorum_members().clone();

        let node_triple = message.into_inner();

        let our_node_id = node.network_view.node_id();

        return match self.current_state {
            JoiningReplicaState::Stable => {
                let network_node = network_node.clone();

                let quorum_view = self.current_view.read().unwrap().clone();

                let predicates = self.predicates.clone();

                match &self.join_node_state {
                    QuorumNodeState::Init | QuorumNodeState::Joining(_, _) => {}
                    _ => {
                        let result = QuorumEnterResponse::Rejected(QuorumEnterRejectionReason::CurrentlyReconfiguring);

                        let quorum_reconfig_msg = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(result));

                        let reconf_message = ReconfigurationMessage::new(seq, quorum_reconfig_msg);

                        let _ = network_node.send_reconfig_message(reconf_message, header.from());

                        return QuorumProtocolResponse::Nil;
                    }
                }

                let mut responses = Vec::new();

                for predicate in predicates {
                    responses.push(predicate(quorum_view.clone(), node_triple.clone()));
                }

                let responses = rt::block_on(join_all(responses));

                for join_result in responses {
                    if let Some(reason) = join_result.unwrap() {
                        let result = QuorumEnterResponse::Rejected(reason);

                        let quorum_reconfig_msg = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(result));

                        let reconf_message = ReconfigurationMessage::new(seq, quorum_reconfig_msg);

                        let _ = network_node.send_reconfig_message(reconf_message, header.from());

                        return QuorumProtocolResponse::Nil;
                    }
                }

                debug!("Node {:?} has been approved to join the quorum", node_triple.node_id());

                self.begin_quorum_join_procedure(node, &network_node, node_triple);

                QuorumProtocolResponse::Nil
            }
            _ => {
                warn!("Received a quorum enter request while not in stable state, ignoring");

                QuorumProtocolResponse::Nil
            }
        };
    }

    fn begin_quorum_join_procedure<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, joining_node: NodeTriple)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let quorum_view = self.current_view.read().unwrap().clone();

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

        network_node.broadcast_reconfig_message(reconf_message, quorum_view.quorum_members().into_iter());
    }
}

impl TboQueue {
    fn init() -> Self {
        Self {
            signal: false,
            joining_message: Default::default(),
            confirm_join_message: Default::default(),
        }
    }

    fn queue(&mut self, seq: SeqNo, header: Header, message: ReconfigurationMessage) {
        match reconfig_quorum_message_type(&message) {
            ReconfigQuorumMessage::JoinMessage(_) => {
                tbo_queue_message(seq, &mut self.joining_message, StoredMessage::new(header, message))
            }
            ReconfigQuorumMessage::ConfirmJoin(_) => {
                tbo_queue_message(seq, &mut self.confirm_join_message, StoredMessage::new(header, message))
            }
        }

        self.signal = true;
    }

    fn has_pending_messages(&self) -> bool {
        self.signal
    }

    fn pop_join_message(&mut self) -> Option<StoredMessage<ReconfigurationMessage>> {
        tbo_pop_message(&mut self.joining_message)
    }

    fn pop_confirm_message(&mut self) -> Option<StoredMessage<ReconfigurationMessage>> {
        tbo_pop_message(&mut self.confirm_join_message)
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

fn necessary_votes(vec: &Vec<NodeId>) -> usize {
    // We need at least 2/3 of the quorum to allow a node join
    let quorum_size = vec.len();

    let necessary_votes = (quorum_size as f64 / 3.0 * 2.0).ceil() as usize;

    necessary_votes
}