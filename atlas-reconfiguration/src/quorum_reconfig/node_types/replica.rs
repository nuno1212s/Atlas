use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use futures::future::join_all;

use log::{debug, error, info};

use atlas_common::async_runtime as rt;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::{Header, StoredMessage};
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use atlas_core::reconfiguration_protocol::{QuorumAlterationResponse, QuorumReconfigurationMessage, QuorumReconfigurationResponse};
use atlas_core::timeouts::Timeouts;

use crate::{GeneralNodeInfo, QuorumProtocolResponse, SeqNoGen, TIMEOUT_DUR};
use crate::message::{QuorumEnterRejectionReason, QuorumEnterRequest, QuorumEnterResponse, QuorumJoinCertificate, QuorumNodeJoinApproval, QuorumReconfigMessage, QuorumViewCert, ReconfData, ReconfigurationMessage, ReconfigurationMessageType};
use crate::quorum_reconfig::{QuorumPredicate, QuorumView};

enum ReplicaState {
    Init,
    // We are currently initializing our state so we know who to contact in order to join the quorum
    Initializing(usize, BTreeSet<NodeId>, BTreeMap<Digest, Vec<QuorumViewCert>>),
    // We are currently attempting to join the quorum
    JoiningQuorum(usize, BTreeSet<NodeId>, BTreeMap<SeqNo, Vec<StoredMessage<QuorumNodeJoinApproval>>>),
    // We are currently stable in the network and are up to date with the quorum status
    Stable,
    // We do not require an Updating state since we receive them directly from the quorum, which already
    // Assures safety
    LeavingQuorum,
}

pub(crate) struct ReplicaQuorumView<JC> {
    /// The current state of the replica
    current_state: ReplicaState,
    /// The current quorum view we know of
    current_view: Arc<RwLock<QuorumView>>,

    /// Predicates that must be satisfied for a node to be allowed to join the quorum
    predicates: Vec<QuorumPredicate>,
    /// Channel to communicate with the ordering protocol
    quorum_communication: ChannelSyncTx<QuorumReconfigurationMessage<JC>>,
    /// Channel to receive responses from the quorum
    quorum_responses: ChannelSyncRx<QuorumReconfigurationResponse>,
}

impl<JC> ReplicaQuorumView<JC> {
    pub fn new(
        quorum_view: Arc<RwLock<QuorumView>>,
        quorum_tx: ChannelSyncTx<QuorumReconfigurationMessage<JC>>,
        quorum_response_rx: ChannelSyncRx<QuorumReconfigurationResponse>,
        predicates: Vec<QuorumPredicate>) -> Self {
        Self {
            current_state: ReplicaState::Init,
            current_view: quorum_view,
            predicates,
            quorum_communication: quorum_tx,
            quorum_responses: quorum_response_rx,
        }
    }

    pub fn iterate<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self.current_state {
            ReplicaState::Init => {
                let reconf_message = QuorumReconfigMessage::NetworkViewStateRequest;

                let known_nodes = node.network_view.known_nodes();

                let contacted_nodes = known_nodes.len();

                let reconfig_message = ReconfigurationMessage::new(seq_no.next_seq(), ReconfigurationMessageType::QuorumReconfig(reconf_message));

                let _ = network_node.broadcast_reconfig_message(reconfig_message, known_nodes.into_iter());

                timeouts.timeout_reconfig_request(TIMEOUT_DUR, (contacted_nodes / 2 + 1) as u32, seq_no.curr_seq());

                self.current_state = ReplicaState::Initializing(contacted_nodes, Default::default(), Default::default());

                QuorumProtocolResponse::Running
            }
            _ => {
                //Nothing to do here
                QuorumProtocolResponse::Nil
            }
        }
    }

    pub fn handle_view_state_message<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo,
                                         network_node: &Arc<NT>, quorum_view_state: QuorumViewCert)
                                         -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            ReplicaState::Init => {
                //Remains from previous initializations?
            }
            ReplicaState::Initializing(sent_messages, received, received_states) => {
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

                debug!("Received {:?} messages out of {:?} needed", received.len(), needed_messages);

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

                    if let Some((quorum_digest, quorum_certs)) = received_messages.first() {
                        if quorum_certs.len() >= needed_messages {
                            {
                                let mut write_guard = self.current_view.write().unwrap();

                                *write_guard = quorum_certs.first().unwrap().message().clone();
                            }

                            return self.start_join_quorum(seq_no, node, network_node);
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

        network_node.send_reconfig_message(ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::NetworkViewState(quorum_view))), header.from());

        QuorumProtocolResponse::Nil
    }

    pub fn handle_timeout<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, timeouts: &Timeouts) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            ReplicaState::Initializing(_, _, _) => {
                let reconf_message = QuorumReconfigMessage::NetworkViewStateRequest;

                let known_nodes = node.network_view.known_nodes();

                let contacted_nodes = known_nodes.len();

                let reconfig_message = ReconfigurationMessage::new(seq_no.next_seq(), ReconfigurationMessageType::QuorumReconfig(reconf_message));

                let _ = network_node.broadcast_reconfig_message(reconfig_message, known_nodes.into_iter());

                self.current_state = ReplicaState::Initializing(contacted_nodes, Default::default(), Default::default());

                QuorumProtocolResponse::Running
            }
            _ => {
                //Nothing to do here
                QuorumProtocolResponse::Nil
            }
        }
    }

    pub fn start_join_quorum<NT>(&mut self, seq_no: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_quorum_members = self.current_view.read().unwrap().quorum_members().clone();

        if current_quorum_members.is_empty() || current_quorum_members.contains(&node.network_view.node_id()) {
            info!("We are already a part of the quorum, moving to stable");

            self.current_state = ReplicaState::Stable;

            QuorumProtocolResponse::Done
        } else {
            info!("Starting join quorum procedure, contacting {:?}", current_quorum_members);

            self.current_state = ReplicaState::JoiningQuorum(current_quorum_members.len(), Default::default(), Default::default());

            let reconf_message = QuorumReconfigMessage::QuorumEnterRequest(QuorumEnterRequest::new(node.network_view.node_triple()));

            let reconfig_message = ReconfigurationMessage::new(seq_no.next_seq(), ReconfigurationMessageType::QuorumReconfig(reconf_message));

            let _ = network_node.broadcast_reconfig_message(reconfig_message, current_quorum_members.into_iter());

            QuorumProtocolResponse::Running
        }
    }

    pub fn handle_quorum_enter_request<NT>(&mut self, seq: SeqNo, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: QuorumEnterRequest) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let quorum_members = self.current_view.read().unwrap().quorum_members().clone();

        let node_triple = message.into_inner();

        let our_node_id = node.network_view.node_id();

        match self.current_state {
            ReplicaState::Stable => {
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

                                network_node.send_reconfig_message(ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(result))), header.from());
                                return;
                            }
                        }

                        // If all predicates pass, then he should be in the clear
                        let join_approval =
                            QuorumNodeJoinApproval::new(quorum_view.sequence_number(),
                                                        node_triple.node_id(),
                                                        our_node_id);

                        let join_approval = QuorumEnterResponse::Successful(join_approval);

                        network_node.send_reconfig_message(ReconfigurationMessage::new(seq, ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumEnterResponse(join_approval))), header.from());
                    });

                    QuorumProtocolResponse::Done
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

    pub fn handle_quorum_enter_response<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, header: Header, message: QuorumEnterResponse) -> QuorumProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            ReplicaState::JoiningQuorum(contacted, received, join_approvals) => {
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

                    let (seq, approvals) = collected_join_approvals.swap_remove(0);

                    if approvals.len() < necessary_response {
                        return QuorumProtocolResponse::Running;
                    }

                    let certificate = QuorumJoinCertificate::new(*seq, approvals.clone());

                    self.quorum_communication.send(QuorumReconfigurationMessage::RequestQuorumJoin(node.network_view.node_id(), certificate));

                    loop {
                        let join_response = self.quorum_responses.recv().unwrap();
                        // Wait for the response from the ordering protocol
                        match join_response {
                            QuorumReconfigurationResponse::QuorumAlterationResponse(response) => {
                                return match response {
                                    QuorumAlterationResponse::Successful => {
                                        self.handle_quorum_entered()
                                    }
                                    QuorumAlterationResponse::Failed() => {
                                        self.start_join_quorum(seq_gen, node, network_node)
                                    }
                                };
                            }
                        }
                    }
                }

                QuorumProtocolResponse::Running
            }
            _ => {
                // We are not currently joining the quorum, so we can ignore this message
                QuorumProtocolResponse::Nil
            }
        }
    }

    pub fn handle_quorum_entered(&mut self) -> QuorumProtocolResponse {
        self.current_state = ReplicaState::Stable;

        return QuorumProtocolResponse::Done;
    }

    pub fn handle_quorum_view_node_added<NT>(&mut self, seq_gen: &mut SeqNoGen, node: &GeneralNodeInfo, network_node: &Arc<NT>, node_added: NodeId)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let mut guard = self.current_view.write().unwrap();

        let novel_view = guard.next_with_added_node(node_added);

        std::mem::replace(&mut *guard, novel_view);

        let reconf_message = ReconfigurationMessageType::QuorumReconfig(QuorumReconfigMessage::QuorumUpdated((*guard).clone()));

        let known_nodes = node.network_view.known_nodes().into_iter().filter(|node| !guard.quorum_members.contains(node));

        // Only send this message to nodes that do not partake in the quorum, as those will be notified of a change by their own ordering protocols
        network_node.broadcast_reconfig_message(ReconfigurationMessage::new(seq_gen.next_seq(), reconf_message), known_nodes);
    }

}