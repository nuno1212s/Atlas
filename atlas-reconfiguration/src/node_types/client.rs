use std::collections::{BTreeMap, BTreeSet};
use log::error;
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_communication::message::StoredMessage;
use atlas_communication::reconfiguration_node::ReconfigurationNode;
use crate::quorum_reconfig::{QuorumView, QuorumNode};
use crate::{ReconfigurableNode, ReplicaQuorumNode};
use crate::message::{QuorumReconfigMessage, QuorumViewCert, ReconfData, ReconfigurationMessage};
use crate::node_types::replica;

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

pub struct ClientQuorumView {
    current_state: ClientState,
    current_quorum_view: QuorumView,

    /// The set of messages that we have received that are part of the current quorum view
    /// That agree on the current
    quorum_view_certificate: Vec<QuorumViewCert>,
}

impl ClientQuorumView {
    pub fn new() -> Self {
        ClientQuorumView {
            current_state: ClientState::Init,
            current_quorum_view: QuorumView::empty(),
            quorum_view_certificate: vec![],
        }
    }

    pub fn iterate<NT>(&mut self, replica: &ReconfigurableNode<NT>)
        where NT: ReconfigurationNode<ReconfData> {
        self.current_state = match self.current_state {
            ClientState::Init => {
                let reconf_message = QuorumReconfigMessage::NetworkViewStateRequest;

                let known_nodes = replica.network_view.known_nodes();

                let _ = replica.network_node.broadcast_reconfig_message(ReconfigurationMessage::QuorumReconfig(reconf_message), known_nodes);

                ClientState::Initializing(known_nodes.len(), Default::default(), Default::default())
            }
            ClientState::Stable => {
                // Nothing to do here
                ClientState::Stable
            }
            ClientState::Initializing(_, _, _) => {}
        }
    }

    /// Handle a view state message being received
    pub fn handle_view_state_message<NT>(&mut self, replica: &ReconfigurableNode<NT>, quorum_view_state: QuorumViewCert)
        where NT: ReconfigurationNode<ReconfData> {
        self.current_state = match self.current_state {
            ClientState::Init => {
                //TODO: Maybe require at least more than one message to be received before we change state?
                ClientState::Init
            }
            ClientState::Initializing(sent_messages, mut received, mut received_message) => {
                // We are still initializing, so we need to add this message to the list of received messages

                if received.insert(quorum_view_state.sender()) {
                    let entry = received_message.entry(quorum_view_state.digest())
                        .or_insert(Default::default());

                    entry.push(quorum_view_state.clone());
                } else {
                    error!("Received duplicate message from node {:?} with digest {:?}",
                           quorum_view_state.sender(), quorum_view_state.digest());
                }

                let needed_messages = sent_messages / 2 + 1;

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
                            self.current_quorum_view = quorum_certs.first().unwrap().quorum_view().clone();

                            self.quorum_view_certificate = quorum_certs.clone();

                            ClientState::Stable
                        } else {
                            ClientState::Initializing(sent_messages, received, received_message)
                        }
                    } else {
                        error!("Received no messages from any nodes");

                        ClientState::Initializing(sent_messages, received, received_message)
                    }
                } else {
                    ClientState::Initializing(sent_messages, received, received_message)
                }
            }
            ClientState::Updating(received, received_messages) => {
                // This type of messages should not be received while we are updating
                ClientState::Updating(received, received_messages)
            }
            ClientState::Stable => {
                // We are already stable, so we don't need to do anything
                ClientState::Stable
            }
        }
    }

    /// Handle a node having entered the quorum view
    pub fn handle_quorum_entered_received(&mut self, replica: &ReplicaQuorumNode, quorum_view_state: QuorumViewCert) {
        self.current_state = match self.current_state {
            ClientState::Init => {
                // We are not ready to handle this message yet
                ClientState::Init
            }
            ClientState::Initializing(a, b, c) => {
                // We are not ready to handle this message yet
                ClientState::Initializing(a, b, c)
            }
            ClientState::Updating(mut received, mut received_message) => {
                if received.insert(quorum_view_state.sender()) {
                    let entry = received_message.entry(quorum_view_state.digest())
                        .or_insert(Default::default());

                    entry.push(quorum_view_state.clone());
                } else {
                    error!("Received duplicate message from node {:?} with digest {:?}",
                           quorum_view_state.sender(), quorum_view_state.digest());
                }

                let needed_messages = (self.current_quorum_view.quorum_members().len() / 3 * 2) + 1;

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
                            self.current_quorum_view = quorum_certs.first().unwrap().quorum_view().clone();

                            self.quorum_view_certificate = quorum_certs.clone();

                            ClientState::Stable
                        } else {
                            ClientState::Updating(received, received_message)
                        }
                    } else {
                        error!("Received no messages from any nodes");

                        ClientState::Updating(received, received_message)
                    }
                } else {
                    ClientState::Updating(received, received_message)
                }
            }
            ClientState::Stable => {



                //TODO: Change to the update state
                ClientState::Stable
            }
        }
    }
}