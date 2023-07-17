#![feature(async_fn_in_trait)]

use std::sync::{Arc, RwLock};
use std::time::Duration;

use log::{info, warn};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::NodeConnections;
use atlas_communication::reconfiguration_node::{ReconfigurationIncomingHandler, ReconfigurationNode};
use atlas_core::reconfiguration_protocol::{QuorumJoinCert, ReconfigResponse, ReconfigurableNodeTypes, ReconfigurationProtocol};
use atlas_core::timeouts::{RqTimeout, TimeoutKind, Timeouts};

use crate::config::ReconfigurableNetworkConfig;
use crate::message::{QuorumJoinCertificate, ReconfData, ReconfigMessage, ReconfigurationMessageType};
use crate::network_reconfig::{GeneralNodeInfo, NetworkInfo, NetworkNodeState};
use crate::quorum_reconfig::node_types::client::ClientQuorumView;
use crate::quorum_reconfig::node_types::NodeType;
use crate::quorum_reconfig::node_types::replica::ReplicaQuorumView;
use crate::quorum_reconfig::QuorumView;

pub mod config;
pub mod message;
pub mod network_reconfig;
pub mod quorum_reconfig;
mod metrics;

const TIMEOUT_DUR: Duration = Duration::from_secs(3);

/// The reconfiguration module.
/// Provides various utilities for allowing reconfiguration of the network
/// Such as message definitions, important types and etc.
///
/// This module will then be used by the parts of the system which must be reconfigurable
/// (For example, the network node, the client node, etc)
#[derive(Debug)]
enum ReconfigurableNodeState {
    NetworkReconfigurationProtocol,
    QuorumReconfigurationProtocol,
    Stable,
}

/// The response returned from iterating the network protocol
pub enum NetworkProtocolResponse {
    Done,
    Running,
    /// Just a response to indicate nothing was done
    Nil,
}

/// The response returned from iterating the quorum protocol
pub enum QuorumProtocolResponse {
    Done,
    Running,
    Nil,
}

/// A reconfigurable node, used to handle the reconfiguration of the network as a whole
pub struct ReconfigurableNode<NT> where NT: Send + 'static {
    seq_gen: SeqNoGen,
    /// The reconfigurable node state
    node_state: ReconfigurableNodeState,
    /// The general information about the network
    node: GeneralNodeInfo,
    /// The reference to the network node
    network_node: Arc<NT>,
    /// Handle to the timeouts module
    timeouts: Timeouts,
    // Receive messages from the other protocols
    channel_rx: ChannelSyncRx<ReconfigMessage>,
    /// The type of the node we are running.
    node_type: NodeType<QuorumJoinCertificate>,
}

#[derive(Debug)]
struct SeqNoGen {
    seq: SeqNo,
}

/// The handle to the current reconfigurable node information.
///
pub struct ReconfigurableNodeProtocol {
    network_info: Arc<NetworkInfo>,
    quorum_info: Arc<RwLock<QuorumView>>,
    channel_tx: ChannelSyncTx<ReconfigMessage>,
}

/// The result of the iteration of the node
#[derive(Clone, Debug)]
enum IterationResult {
    ReceiveMessage,
    Idle,
}

impl SeqNoGen {
    pub fn curr_seq(&self) -> SeqNo {
        self.seq
    }

    pub fn next_seq(&mut self) -> SeqNo {
        self.seq += SeqNo::ONE;

        self.seq
    }
}

impl<NT> ReconfigurableNode<NT> where NT: Send + 'static {
    fn switch_state(&mut self, new_state: ReconfigurableNodeState) {

        match (&self.node_state, &new_state) {
            (ReconfigurableNodeState::NetworkReconfigurationProtocol, ReconfigurableNodeState::QuorumReconfigurationProtocol) => {
                info!("We have finished the network reconfiguration protocol, running the quorum reconfiguration message");
            }
            (ReconfigurableNodeState::QuorumReconfigurationProtocol, ReconfigurableNodeState::Stable) => {
                info!("We have finished the quorum reconfiguration protocol, switching to stable.");
            }
            (_, _) => {
                warn!("Illegal transition of states, {:?} to {:?}", self.node_state, new_state);

                return;
            }
        }

        self.node_state = new_state;
    }

    fn run(mut self) where NT: ReconfigurationNode<ReconfData> + 'static {
        loop {
            self.handle_local_messages();

            match self.node_state {
                ReconfigurableNodeState::NetworkReconfigurationProtocol => {
                    match self.node.iterate(&mut self.seq_gen, &self.network_node, &self.timeouts) {
                        NetworkProtocolResponse::Done => {
                            self.switch_state(ReconfigurableNodeState::QuorumReconfigurationProtocol);
                        }
                        NetworkProtocolResponse::Running => {}
                        NetworkProtocolResponse::Nil => {}
                    };
                }
                ReconfigurableNodeState::QuorumReconfigurationProtocol => {
                    match self.node_type.iterate(&mut self.seq_gen, &self.node, &self.network_node, &self.timeouts) {
                        QuorumProtocolResponse::Done => {
                            self.switch_state(ReconfigurableNodeState::Stable);
                        }
                        QuorumProtocolResponse::Running => {}
                        QuorumProtocolResponse::Nil => {}
                    };
                }
                ReconfigurableNodeState::Stable => {}
            }

            let optional_message = self.network_node.reconfiguration_message_handler().try_receive_reconfig_message().unwrap();

            if let Some(message) = optional_message {
                let (header, message) = message.into_inner();

                let (seq, message) = message.into_inner();

                match message {
                    ReconfigurationMessageType::NetworkReconfig(network_reconfig) => {
                        match self.node.handle_network_reconfig_msg(&mut self.seq_gen, &self.network_node, &self.timeouts, header, seq, network_reconfig) {
                            NetworkProtocolResponse::Done => {
                                self.switch_state(ReconfigurableNodeState::QuorumReconfigurationProtocol);
                            }
                            NetworkProtocolResponse::Running => {}
                            NetworkProtocolResponse::Nil => {}
                        };
                    }
                    ReconfigurationMessageType::QuorumReconfig(quorum_reconfig) => {
                        match self.node_type.handle_reconfigure_message(&mut self.seq_gen, &self.node, &self.network_node, header, seq,quorum_reconfig) {
                            QuorumProtocolResponse::Done => {
                                self.switch_state(ReconfigurableNodeState::Stable);
                            }
                            QuorumProtocolResponse::Running => {}
                            QuorumProtocolResponse::Nil => {}
                        };
                    }
                }
            }
        }
    }

    fn handle_local_messages(&mut self) where NT: ReconfigurationNode<ReconfData> + 'static {
        while let Ok(received_message) = self.channel_rx.try_recv() {
            match received_message {
                ReconfigMessage::TimeoutReceived(timeout) => {
                    info!("We have received timeouts {:?}", timeout);

                    for rq_timeout in timeout {
                        match rq_timeout.timeout_kind() {
                            TimeoutKind::Reconfiguration(seq) => {
                                if *seq != self.seq_gen.curr_seq() {
                                    warn!("Received a timeout with an invalid sequence number {:?} vs {:?} (Ours)", seq, self.seq_gen);
                                    continue;
                                }

                                match self.node_state {
                                    ReconfigurableNodeState::NetworkReconfigurationProtocol => {
                                        self.node.handle_timeout(&mut self.seq_gen, &self.network_node, &self.timeouts);
                                    }
                                    ReconfigurableNodeState::QuorumReconfigurationProtocol => {}
                                    ReconfigurableNodeState::Stable => {
                                        info!("Received a reconfiguration timeout while we are stable, this does not make sense");
                                    }
                                }

                                self.timeouts.cancel_reconfig_timeout(Some(*seq));
                            }
                            _ => unreachable!("Received a timeout that is not a reconfiguration timeout")
                        }
                    }
                }
            }
        }
    }
}

impl ReconfigurationProtocol for ReconfigurableNodeProtocol {
    type Config = ReconfigurableNetworkConfig;
    type InformationProvider = NetworkInfo;
    type Serialization = ReconfData;

    fn init_default_information(config: Self::Config) -> Result<Arc<Self::InformationProvider>> {
        Ok(Arc::new(NetworkInfo::init_from_config(config)))
    }

    async fn initialize_protocol<NT>(information: Arc<Self::InformationProvider>,
                                     node: Arc<NT>, timeouts: Timeouts,
                                     node_type: ReconfigurableNodeTypes<QuorumJoinCert<Self::Serialization>>)
                                     -> Result<Self> where NT: ReconfigurationNode<Self::Serialization> + 'static, Self: Sized {
        let general_info = GeneralNodeInfo::new(information.clone(), NetworkNodeState::Init);

        let quorum_view = Arc::new(RwLock::new(QuorumView::empty()));

        let node_type = match node_type {
            ReconfigurableNodeTypes::Client => {
                NodeType::Client(ClientQuorumView::new(quorum_view.clone()))
            }
            ReconfigurableNodeTypes::Replica(channel_tx, channel_rx) => {
                NodeType::Replica(ReplicaQuorumView::new(quorum_view.clone(), channel_tx, channel_rx))
            }
        };

        let (channel_tx, channel_rx) = channel::new_bounded_sync(128);

        let reconfigurable_node = ReconfigurableNode {
            seq_gen: SeqNoGen { seq: SeqNo::ZERO },
            node_state: ReconfigurableNodeState::NetworkReconfigurationProtocol,
            node: general_info,
            network_node: node.clone(),
            timeouts,
            channel_rx,
            node_type,
        };

        std::thread::Builder::new()
            .name(format!("Reconfiguration Protocol Thread"))
            .spawn(move || {
                reconfigurable_node.run();
            }).expect("Failed to launch reconfiguration protocol thread");

        let node_handle = ReconfigurableNodeProtocol {
            network_info: information.clone(),
            quorum_info: quorum_view,
            channel_tx,
        };

        Ok(node_handle)
    }

    fn handle_timeout(&self, timeouts: Vec<RqTimeout>) -> Result<ReconfigResponse> {
        self.channel_tx.send(ReconfigMessage::TimeoutReceived(timeouts)).unwrap();

        Ok(ReconfigResponse::Running)
    }

    fn get_quorum_members(&self) -> Vec<NodeId> {
        self.quorum_info.read().unwrap().quorum_members().clone()
    }

    fn is_join_certificate_valid(&self, certificate: &QuorumJoinCert<Self::Serialization>) -> bool {
        todo!()
    }
}
