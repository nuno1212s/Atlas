use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use log::error;
use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use crate::message::{QuorumEnterRequest, QuorumNodeJoinApproval, QuorumReconfigMessage, QuorumReconfigurationMessage, QuorumReconfigurationResponse, QuorumViewCert, ReconfData, ReconfigurationMessage};
use crate::quorum_reconfig::QuorumView;
use crate::GeneralNodeInfo;

enum ReplicaState {
    Init,
    // We are currently initializing our state so we know who to contact in order to join the quorum
    Initializing(usize, BTreeSet<NodeId>, BTreeMap<Digest, Vec<QuorumViewCert>>),
    // We are currently attempting to join the quorum
    JoiningQuorum(usize, BTreeSet<NodeId>, BTreeMap<SeqNo, Vec<QuorumNodeJoinApproval>>),
    // We are currently stable in the network and are up to date with the quorum status
    Stable,
    // We do not require an Updating state since we receive them directly from the quorum, which already
    // Assures safety
    LeavingQuorum,
}

pub(crate) struct ReplicaQuorumView {
    /// The current state of the replica
    current_state: ReplicaState,
    /// The current quorum view we know of
    current_view: QuorumView,

    quorum_communication: ChannelSyncTx<QuorumReconfigurationMessage>,
    quorum_responses: ChannelSyncRx<QuorumReconfigurationResponse>,
}

impl ReplicaQuorumView {
    pub fn new() -> Self {
        let (quorum_tx, quorum_rx) = channel::new_bounded_sync(128);
        let (quorum_response_tx, quorum_response_rx) = channel::new_bounded_sync(128);

        Self {
            current_state: ReplicaState::Init,
            current_view: QuorumView::empty(),
            quorum_communication: quorum_tx,
            quorum_responses: quorum_response_rx,
        }
    }

    pub fn iterate<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match self.current_state {
            ReplicaState::Init => {
                let reconf_message = QuorumReconfigMessage::NetworkViewStateRequest;

                let known_nodes = node.network_view.known_nodes();

                let contacted_nodes = known_nodes.len();

                let _ = network_node.broadcast_reconfig_message(ReconfigurationMessage::QuorumReconfig(reconf_message), known_nodes.into_iter());

                self.current_state = ReplicaState::Initializing(contacted_nodes, Default::default(), Default::default())
            }
            _ => {
                //Nothing to do here
            }
        }
    }

    pub fn handle_view_state_message<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, quorum_view_state: QuorumViewCert)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            ReplicaState::Init => {
                //Remains from previous initializations?
            }
            ReplicaState::Initializing(sent_messages, received, received_states) => {
                // We are still initializing

                if received.insert(quorum_view_state.sender()) {
                    let entry = received_states.entry(quorum_view_state.digest())
                        .or_insert(Default::default());

                    entry.push(quorum_view_state.clone());
                } else {
                    error!("Received duplicate message from node {:?} with digest {:?}",
                           quorum_view_state.sender(), quorum_view_state.digest());
                }

                let needed_messages = *sent_messages / 2 + 1;

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
                            self.current_view = quorum_certs.first().unwrap().quorum_view().clone();

                            self.start_join_quorum(node, network_node);
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
    }

    pub fn start_join_quorum<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let current_quorum_members = self.current_view.quorum_members().clone();

        self.current_state = ReplicaState::JoiningQuorum(current_quorum_members.len(), Default::default(), Default::default());

        let message = QuorumReconfigMessage::QuorumEnterRequest(QuorumEnterRequest::new(node.network_view.node_triple()));

        let _ = network_node.broadcast_reconfig_message(ReconfigurationMessage::QuorumReconfig(message), current_quorum_members.into_iter());
    }
}