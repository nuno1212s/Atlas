use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use log::error;

use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_communication::reconfiguration_node::ReconfigurationNode;

use crate::{GeneralNodeInfo};
use crate::message::{QuorumReconfigMessage, QuorumViewCert, ReconfData, ReconfigurationMessage};
use crate::quorum_reconfig::QuorumView;

enum ClientState {
    /// We are initializing our state
    Init,
    /// We are receiving quorum information from the network
    Initializing(usize, BTreeSet<NodeId>, BTreeMap<Digest, Vec<QuorumViewCert>>),
    /// We already have a quorum view, but we are in the middle of receiving a new state from the quorum
    Updating(BTreeSet<NodeId>, BTreeMap<Digest, Vec<QuorumViewCert>>),
    /// We are up to date with the quorum members
    Stable,

}

///
pub(crate) struct ClientQuorumView {
    current_state: ClientState,
    current_quorum_view: Arc<RwLock<QuorumView>>,

    /// The set of messages that we have received that are part of the current quorum view
    /// That agree on the current
    quorum_view_certificate: Vec<QuorumViewCert>,
}

impl ClientQuorumView {
    pub fn new(quorum_view: Arc<RwLock<QuorumView>>) -> Self {
        ClientQuorumView {
            current_state: ClientState::Init,
            current_quorum_view: quorum_view,
            quorum_view_certificate: vec![],
        }
    }

    pub fn iterate<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>)
        where NT: ReconfigurationNode<ReconfData> {
        match &mut self.current_state {
            ClientState::Init => {
                let reconf_message = QuorumReconfigMessage::NetworkViewStateRequest;

                let known_nodes = node.network_view.known_nodes();

                let contacted_nodes = known_nodes.len();

                let _ = network_node.broadcast_reconfig_message(ReconfigurationMessage::QuorumReconfig(reconf_message), known_nodes.into_iter());

                self.current_state = ClientState::Initializing(contacted_nodes, Default::default(), Default::default())
            }
            _ => {
                //Nothing to do here
            }
        }
    }

    /// Handle a view state message being received
    pub fn handle_view_state_message<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>, quorum_view_state: QuorumViewCert)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            ClientState::Init => {
                //TODO: Maybe require at least more than one message to be received before we change state?
            }
            ClientState::Initializing(sent_messages, received, received_message) => {
                // We are still initializing, so we need to add this message to the list of received messages

                if received.insert(quorum_view_state.sender()) {
                    let entry = received_message.entry(quorum_view_state.digest())
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

                    for (message_digests, messages) in received_message.iter() {
                        received_messages.push((message_digests.clone(), messages.clone()));
                    }

                    received_messages.sort_by(|(_, a), (_, b)| {
                        a.len().cmp(&b.len()).reverse()
                    });

                    if let Some((quorum_digest, quorum_certs)) = received_messages.first() {
                        if quorum_certs.len() >= needed_messages {
                            {
                                let mut write_guard = self.current_quorum_view.write().unwrap();

                                *write_guard = quorum_certs.first().unwrap().quorum_view().clone();
                            }

                            self.quorum_view_certificate = quorum_certs.clone();

                            self.current_state = ClientState::Stable;
                        }
                    } else {
                        error!("Received no messages from any nodes");
                    }
                }
            }
            ClientState::Updating(received, received_messages) => {
                // This type of messages should not be received while we are updating
            }
            ClientState::Stable => {
                // We are already stable, so we don't need to do anything
            }
        }
    }

    /// Handle a node having entered the quorum view
    pub fn handle_quorum_entered_received<NT>(&mut self, node: &GeneralNodeInfo, network_node: &Arc<NT>,
                                              quorum_view_state: QuorumViewCert)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            ClientState::Init => {
                // We are not ready to handle this message yet
            }
            ClientState::Initializing(a, b, c) => {
                // We are not ready to handle this message yet
            }
            ClientState::Updating(received, received_message) => {
                if received.insert(quorum_view_state.sender()) {
                    let entry = received_message.entry(quorum_view_state.digest())
                        .or_insert(Default::default());

                    entry.push(quorum_view_state.clone());
                } else {
                    error!("Received duplicate message from node {:?} with digest {:?}",
                           quorum_view_state.sender(), quorum_view_state.digest());
                }

                let needed_messages = (self.current_quorum_view.read().unwrap().quorum_members().len() / 3 * 2) + 1;

                if received.len() >= needed_messages {
                    // We have received all of the messages that we are going to receive, so we can now
                    // determine the current quorum view

                    let mut received_messages = Vec::new();

                    for (message_digests, messages) in received_message.iter() {
                        received_messages.push((message_digests.clone(), messages.clone()));
                    }

                    received_messages.sort_by(|(_, a), (_, b)| {
                        a.len().cmp(&b.len()).reverse()
                    });

                    if let Some((quorum_digest, quorum_certs)) = received_messages.first() {
                        if quorum_certs.len() >= needed_messages {
                            {
                                let mut write_guard = self.current_quorum_view.write().unwrap();
                                *write_guard = quorum_certs.first().unwrap().quorum_view().clone();
                            }

                            self.quorum_view_certificate = quorum_certs.clone();

                            self.current_state = ClientState::Stable;
                        }
                    } else {
                        error!("Received no messages from any nodes");
                    }
                }
            }
            ClientState::Stable => {
                if self.current_quorum_view.read().unwrap().sequence_number() < quorum_view_state.quorum_view().sequence_number() {
                    // We have received a message from a node that is not in the current quorum view
                    // so we need to update the quorum view

                    let mut received = BTreeSet::new();

                    received.insert(quorum_view_state.sender());

                    let mut received_message = BTreeMap::new();

                    let entry: &mut Vec<QuorumViewCert> = received_message.entry(quorum_view_state.digest())
                        .or_insert(Default::default());

                    entry.push(quorum_view_state.clone());

                    self.current_state = ClientState::Updating(received, received_message);
                }

                //TODO: Change to the update state
            }
        }
    }
}