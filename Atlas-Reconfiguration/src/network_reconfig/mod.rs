use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use futures::future::join_all;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use atlas_common::channel::oneshot::OneShotRx;
use atlas_common::channel::sync::ChannelSyncTx;
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::error::*;
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::ordering::SeqNo;
use atlas_common::peer_addr::PeerAddr;
use atlas_common::{async_runtime as rt, quiet_unwrap, threadpool};
use atlas_communication::byte_stub::connections::NetworkConnectionController;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration::{
    NetworkInformationProvider, NodeInfo, ReconfigurationNetworkCommunication,
    ReconfigurationNetworkUpdate, ReconfigurationNetworkUpdateMessage,
};
use atlas_communication::stub::{ModuleOutgoingStub, RegularNetworkStub};
use atlas_core::reconfiguration_protocol::NodeConnectionUpdateMessage;
use atlas_core::timeouts::timeout::TimeoutModHandle;
use atlas_core::timeouts::TimeoutID;

use crate::config::ReconfigurableNetworkConfig;
use crate::message::{
    signatures, KnownNodesMessage, NetworkJoinCert, NetworkJoinRejectionReason,
    NetworkJoinResponseMessage, NetworkReconfigMessage, NetworkReconfigMessageType, ReconfData,
    ReconfigurationMessage,
};
use crate::{NetworkProtocolResponse, SeqNoGen, TIMEOUT_DUR};

/// The reconfiguration module.
/// Provides various utilities for allowing reconfiguration of the network
/// Such as message definitions, important types and etc.
///
/// This module will then be used by the parts of the system which must be reconfigurable
pub type NetworkPredicate =
    fn(Arc<NetworkInfo>, NodeInfo) -> OneShotRx<Option<NetworkJoinRejectionReason>>;

/// Our current view of the network and the information about our own node
/// This is the node data for the network information. This does not
/// directly store information about the quorum, only about the nodes that we
/// currently know about
pub struct NetworkInfo {
    // The information about our own node
    node_info: NodeInfo,
    key_pair: Arc<KeyPair>,

    // The list of bootstrap nodes that we initially knew in the network.
    // This will be used to verify attempted node joins
    bootstrap_nodes: Vec<NodeId>,

    // The list of nodes that we currently know in the network
    known_nodes: RwLock<KnownNodes>,

    /// Predicates that must be satisfied for a node to be allowed to join the network
    predicates: Vec<NetworkPredicate>,
}

impl NetworkInfo {
    pub fn init_from_config(config: ReconfigurableNetworkConfig) -> Self {
        let ReconfigurableNetworkConfig {
            node_id,
            node_type,
            key_pair,
            our_address,
            known_nodes,
        } = config;

        let boostrap_nodes = known_nodes.iter().map(|triple| triple.node_id()).collect();

        NetworkInfo {
            node_info: NodeInfo::new(
                node_id,
                node_type,
                key_pair.public_key().into(),
                our_address.clone(),
            ),
            key_pair,
            bootstrap_nodes: boostrap_nodes,
            known_nodes: RwLock::new(KnownNodes::from_known_list(known_nodes)),
            predicates: Vec::new(),
        }
    }

    pub fn empty_network_node(
        node_id: NodeId,
        node_type: NodeType,
        key_pair: KeyPair,
        address: PeerAddr,
    ) -> Self {
        NetworkInfo {
            node_info: NodeInfo::new(node_id, node_type, key_pair.public_key().into(), address),
            key_pair: Arc::new(key_pair),
            known_nodes: RwLock::new(KnownNodes::empty()),
            predicates: vec![],
            bootstrap_nodes: vec![],
        }
    }

    /// Initialize a NetworkNode with a list of already known Nodes so we can bootstrap our information
    /// From them.
    pub fn with_bootstrap_nodes(
        node_id: NodeId,
        key_pair: KeyPair,
        node_type: NodeType,
        address: PeerAddr,
        bootstrap_nodes: BTreeMap<NodeId, (PeerAddr, NodeType, Vec<u8>)>,
    ) -> Self {
        let node = NetworkInfo::empty_network_node(node_id, node_type, key_pair, address);

        {
            let mut write_guard = node.known_nodes.write().unwrap();

            for (node_id, (addr, node_type, pk_bytes)) in bootstrap_nodes {
                let public_key = PublicKey::from_bytes(&pk_bytes[..]).unwrap();

                let info = NodeInfo::new(node_id, node_type, public_key, addr);

                write_guard.node_info.insert(node_id, info);
            }
        }

        node
    }

    pub fn register_join_predicate(&mut self, predicate: NetworkPredicate) {
        self.predicates.push(predicate)
    }

    pub fn node_id(&self) -> NodeId {
        self.node_info.node_id()
    }

    /// Handle a node having introduced itself to us by inserting it into our known nodes
    pub(crate) fn handle_node_introduced(&self, node: NodeInfo) -> bool {
        debug!(
            "Handling a node having been introduced {:?}. Handling it",
            node
        );

        let mut write_guard = self.known_nodes.write().unwrap();

        Self::handle_single_node_introduced(&mut write_guard, node)
    }

    pub(crate) fn is_valid_network_hello(
        &self,
        node: NodeInfo,
        certificates: Vec<NetworkJoinCert>,
    ) -> std::result::Result<(), ValidNetworkHelloError> {
        debug!(
            "Received a node hello message from node {:?}. Handling it",
            node
        );

        for (from, signature) in &certificates {
            let from_pk = self.get_pk_for_node(from);
            if let Some(pk) = from_pk {
                if !signatures::verify_node_triple_signature(&node, signature, &pk) {
                    error!("Received a node hello message from node {:?} with invalid signature. Ignoring it",node);

                    return Err(ValidNetworkHelloError::InvalidSignatures);
                }
            } else {
                error!("Received a node hello message from node {:?} with certificate from node {:?} which we don't know. Ignoring it",node, from);
            }

            if !self.bootstrap_nodes.contains(from) {
                error!("Received a node hello message from node {:?} with certificate from node {:?} which is not a bootstrap node. Ignoring it",node, from);
                return Err(ValidNetworkHelloError::NonBootstrapCertificate);
            }
        }

        let mut required = (self.bootstrap_nodes.len() * 2 / 3) + 1;

        if self.bootstrap_nodes.contains(&node.node_id()) {
            // If the node is a bootstrap node, then we don't need one of the nodes
            required -= 1;
        }

        if certificates.len() < required {
            error!("Received a node hello message from node {:?} with less certificates than 2n/3 bootstrap nodes {:?} vs required {:?}. Ignoring it", node, certificates.len(), required);
            return Err(ValidNetworkHelloError::NotEnoughCertificates {
                required,
                provided: certificates.len(),
            });
        }

        Ok(())
    }

    /// Handle us having received a successfull network join response, with the list of known nodes
    /// Returns a list of nodes that we didn't know before, but that have now been added
    pub(crate) fn handle_received_network_view(
        &self,
        known_nodes: KnownNodesMessage,
    ) -> Vec<NodeInfo> {
        let mut write_guard = self.known_nodes.write().unwrap();

        debug!(
            "Updating our known nodes list with the received list {:?}",
            known_nodes
        );

        let mut new_nodes = Vec::with_capacity(known_nodes.known_nodes().len());

        for node in known_nodes.into_nodes() {
            let node_id = node.node_id();

            if node_id == self.node_id() {
                continue;
            }

            if Self::handle_single_node_introduced(&mut write_guard, node.clone()) {
                new_nodes.push(node);
            }
        }

        new_nodes
    }

    fn handle_single_node_introduced(write_guard: &mut KnownNodes, node: NodeInfo) -> bool {
        let node_id = node.node_id();

        if let Entry::Vacant(e) = write_guard.node_info.entry(node_id) {
            e.insert(node);

            true
        } else {
            debug!(
                "Node {:?} has already been introduced to us. Ignoring",
                node
            );

            false
        }
    }

    /// Can we introduce this node to the network
    pub async fn can_introduce_node(
        self: &Arc<Self>,
        node_id: NodeInfo,
    ) -> NetworkJoinResponseMessage {
        let mut results = Vec::with_capacity(self.predicates.len());

        for x in &self.predicates {
            let rx = x(self.clone(), node_id.clone());

            results.push(rx);
        }

        let results = join_all(results.into_iter()).await;

        for join_result in results {
            if let Some(reason) = join_result.unwrap() {
                return NetworkJoinResponseMessage::Rejected(reason);
            }
        }

        let signature = signatures::create_node_triple_signature(&node_id, &self.key_pair)
            .expect("Failed to sign node triple");

        let read_guard = self.known_nodes.read().unwrap();

        NetworkJoinResponseMessage::Successful(signature, KnownNodesMessage::from(&*read_guard))
    }

    pub fn get_pk_for_node(&self, node: &NodeId) -> Option<PublicKey> {
        self.known_nodes
            .read()
            .unwrap()
            .node_info()
            .get(node)
            .map(|info| info.public_key())
            .cloned()
    }

    pub fn get_addr_for_node(&self, node: &NodeId) -> Option<PeerAddr> {
        self.known_nodes
            .read()
            .unwrap()
            .node_info()
            .get(node)
            .map(|info| info.addr())
            .cloned()
    }

    pub fn get_own_addr(&self) -> &PeerAddr {
        self.node_info.addr()
    }

    pub fn keypair(&self) -> &Arc<KeyPair> {
        &self.key_pair
    }

    pub fn known_nodes(&self) -> Vec<NodeId> {
        self.known_nodes
            .read()
            .unwrap()
            .node_info()
            .keys()
            .cloned()
            .collect()
    }

    pub fn known_nodes_and_types(&self) -> Vec<(NodeId, NodeType)> {
        self.known_nodes
            .read()
            .unwrap()
            .node_info()
            .iter()
            .map(|(node_id, info)| (*node_id, info.node_type()))
            .collect()
    }

    pub fn node_triple(&self) -> NodeInfo {
        self.node_info.clone()
    }

    pub fn bootstrap_nodes(&self) -> &Vec<NodeId> {
        &self.bootstrap_nodes
    }
}

impl NetworkInformationProvider for NetworkInfo {
    fn own_node_info(&self) -> &NodeInfo {
        &self.node_info
    }

    fn get_key_pair(&self) -> &Arc<KeyPair> {
        &self.key_pair
    }

    fn get_node_info(&self, node: &NodeId) -> Option<NodeInfo> {
        self.known_nodes
            .read()
            .unwrap()
            .node_info()
            .get(node)
            .cloned()
    }
}

/// The map of known nodes in the network, independently of whether they are part of the current
/// quorum or not
#[derive(Clone)]
pub struct KnownNodes {
    pub(crate) node_info: BTreeMap<NodeId, NodeInfo>,
}

impl KnownNodes {
    fn empty() -> Self {
        Self {
            node_info: Default::default(),
        }
    }

    fn from_known_list(nodes: Vec<NodeInfo>) -> Self {
        let mut known_nodes = Self::empty();

        for node in nodes {
            NetworkInfo::handle_single_node_introduced(&mut known_nodes, node);
        }

        known_nodes
    }

    pub(crate) fn node_info(&self) -> &BTreeMap<NodeId, NodeInfo> {
        &self.node_info
    }
}

/// The current state of our node (network level, not quorum level)
/// Quorum level operations will only take place when we are a stable member of the protocol
#[derive(Debug, Clone)]
pub enum NetworkNodeState {
    /// The node is currently initializing and will attempt to join the network
    Init,
    /// We have broadcast the join request to the known nodes and are waiting for their responses
    /// Which contain the network information. Afterwards, we will attempt to introduce ourselves to all
    /// nodes in the network (if there are more nodes than the known boostrap nodes)
    JoiningNetwork {
        contacted: usize,
        responded: BTreeSet<NodeId>,
        certificates: Vec<NetworkJoinCert>,
    },
    /// We are currently introducing ourselves to the network (and attempting to acquire all known nodes)
    IntroductionPhase {
        contacted: usize,
        responded: BTreeSet<NodeId>,
    },
    /// A stable member of the network, up to date with the current membership
    StableMember,
    /// We are currently leaving the network
    LeavingNetwork,
}

/// The reconfigurable node which will handle all reconfiguration requests
/// This handles the network level reconfiguration, not the quorum level reconfiguration
pub struct GeneralNodeInfo {
    /// Our current view of the network, including nodes we know about, their addresses and public keys
    pub(crate) network_view: Arc<NetworkInfo>,
    /// The current state of the network node, to keep track of which protocols we are executing
    current_state: NetworkNodeState,
    /// The network update channel to send updates about newly discovered nodes
    network_update: ChannelSyncTx<NodeConnectionUpdateMessage>,
}

impl GeneralNodeInfo {
    /// Attempt to iterate and move our current state forward
    pub(super) fn iterate<NT>(
        &mut self,
        seq: &mut SeqNoGen,
        network_node: &Arc<NT>,
        timeouts: &TimeoutModHandle,
    ) -> Result<NetworkProtocolResponse>
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        match &mut self.current_state {
            NetworkNodeState::Init => {
                let known_nodes: Vec<NodeId> = self
                    .network_view
                    .known_nodes()
                    .into_iter()
                    .filter(|node| *node != self.network_view.node_id())
                    .collect();

                let join_req = NetworkReconfigMessage::new(
                    seq.next_seq(),
                    NetworkReconfigMessageType::NetworkJoinRequest(self.network_view.node_triple()),
                );

                let join_message = ReconfigurationMessage::NetworkReconfig(join_req);

                let contacted = known_nodes.len();

                if known_nodes.is_empty() {
                    info!("No known nodes, joining network as a stable member");
                    self.current_state = NetworkNodeState::StableMember;
                }

                let mut node_results = Vec::new();

                for node in &known_nodes {
                    info!(
                        "{:?} // Connecting to node {:?}",
                        self.network_view.node_id(),
                        node
                    );

                    let node_connection_results = network_node.connections().connect_to_node(*node);

                    node_results.push((*node, node_connection_results));
                }

                for (node, conn_results) in node_results {
                    let conn_results =
                        quiet_unwrap!(conn_results, Ok(NetworkProtocolResponse::Nil));

                    for conn_result in conn_results {
                        if let Err(err) = conn_result.recv()? {
                            error!("Error while connecting to another node: {:?}", err);
                        }
                    }

                    info!(
                        "{:?} // Connected to node {:?}",
                        self.network_view.node_id(),
                        node
                    );
                }

                let res = network_node
                    .outgoing_stub()
                    .broadcast_signed(join_message, known_nodes.clone().into_iter());

                info!(
                    "Broadcasting reconfiguration network join message to known nodes {:?}, {:?}",
                    known_nodes, res
                );

                let _ = timeouts.request_timeout(
                    TimeoutID::SeqNoBased(seq.curr_seq()),
                    None,
                    TIMEOUT_DUR,
                    (contacted * 2 / 3) + 1,
                    false,
                );

                self.move_to_joining(contacted);

                return Ok(NetworkProtocolResponse::Nil);
            }
            NetworkNodeState::IntroductionPhase { .. } => {}
            NetworkNodeState::JoiningNetwork { .. } => {}
            NetworkNodeState::StableMember => {}
            NetworkNodeState::LeavingNetwork => {}
        }

        Ok(NetworkProtocolResponse::Nil)
    }

    pub(super) fn handle_timeout<NT>(
        &mut self,
        seq_gen: &mut SeqNoGen,
        network_node: &Arc<NT>,
        timeouts: &TimeoutModHandle,
    ) -> NetworkProtocolResponse
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        match &mut self.current_state {
            NetworkNodeState::JoiningNetwork { .. } => {
                info!("Joining network timeout triggered");

                let known_nodes: Vec<NodeId> = self
                    .network_view
                    .known_nodes()
                    .into_iter()
                    .filter(|node| *node != self.network_view.node_id())
                    .collect();

                let contacted = known_nodes.len();

                if known_nodes.is_empty() {
                    info!("No known nodes, joining network as a stable member");
                    self.current_state = NetworkNodeState::StableMember;

                    return NetworkProtocolResponse::Done;
                }

                let mut node_results = Vec::new();

                for node in &known_nodes {
                    if network_node.connections().has_connection(node) {
                        continue;
                    }

                    info!(
                        "{:?} // Connecting to node {:?}",
                        self.network_view.node_id(),
                        node
                    );

                    let node_connection_results = network_node.connections().connect_to_node(*node);

                    node_results.push((*node, node_connection_results));
                }

                for (_node, conn_results) in node_results {
                    match conn_results {
                        Ok(conn_results) => {
                            for conn_result in conn_results {
                                if let Err(err) = conn_result.recv().unwrap() {
                                    error!("Error while connecting to another node: {:?}", err);
                                }
                            }
                        }
                        Err(_) => return NetworkProtocolResponse::Nil,
                    }
                }

                let join_req = NetworkReconfigMessage::new(
                    seq_gen.next_seq(),
                    NetworkReconfigMessageType::NetworkJoinRequest(self.network_view.node_triple()),
                );

                let join_message = ReconfigurationMessage::NetworkReconfig(join_req);

                info!("Received timeout, broadcasting reconfiguration network join message to known nodes {:?}", known_nodes);

                let _ = network_node
                    .outgoing_stub()
                    .broadcast_signed(join_message, known_nodes.into_iter());

                let _ = timeouts.request_timeout(
                    TimeoutID::SeqNoBased(seq_gen.curr_seq()),
                    None,
                    TIMEOUT_DUR,
                    (contacted * 2 / 3) + 1,
                    false,
                );

                self.current_state = NetworkNodeState::JoiningNetwork {
                    contacted,
                    responded: Default::default(),
                    certificates: Default::default(),
                };

                return NetworkProtocolResponse::Nil;
            }
            NetworkNodeState::IntroductionPhase { .. } => {}
            NetworkNodeState::Init => {}
            NetworkNodeState::StableMember => {}
            NetworkNodeState::LeavingNetwork => {}
        }

        NetworkProtocolResponse::Nil
    }

    pub(super) fn is_response_to_request(
        &self,
        _seq_gen: &SeqNoGen,
        _header: &Header,
        _seq: SeqNo,
        message: &NetworkReconfigMessageType,
    ) -> bool {
        matches!(
            message,
            NetworkReconfigMessageType::NetworkJoinResponse(_)
                | NetworkReconfigMessageType::NetworkHelloReply(_)
        )
    }

    pub(super) fn handle_network_reconfig_msg<NT>(
        &mut self,
        seq_gen: &mut SeqNoGen,
        network_node: &Arc<NT>,
        reconf_msg_handler: &ReconfigurationNetworkCommunication,
        timeouts: &TimeoutModHandle,
        header: Header,
        message: NetworkReconfigMessage,
    ) -> NetworkProtocolResponse
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        let (seq, message) = message.into_inner();

        match &mut self.current_state {
            NetworkNodeState::JoiningNetwork {
                contacted,
                responded,
                certificates,
            } => {
                // Avoid accepting double answers
                match message {
                    NetworkReconfigMessageType::NetworkJoinRequest(join_request) => {
                        info!(
                            "Received a network join request from {:?} while joining the network",
                            header.from()
                        );

                        self.handle_join_request(
                            network_node,
                            reconf_msg_handler,
                            header,
                            seq,
                            join_request,
                        )
                    }
                    NetworkReconfigMessageType::NetworkJoinResponse(join_response) => {
                        if responded.insert(header.from()) {
                            match join_response {
                                NetworkJoinResponseMessage::Successful(
                                    signature,
                                    network_information,
                                ) => {
                                    info!("We were accepted into the network by the node {:?}, current certificate count {:?}", header.from(), certificates.len());

                                    let _ = timeouts.cancel_timeout(TimeoutID::SeqNoBased(seq));

                                    let unknown_nodes = self
                                        .network_view
                                        .handle_received_network_view(network_information);

                                    for node_id in unknown_nodes {
                                        if !network_node
                                            .connections()
                                            .has_connection(&node_id.node_id())
                                        {
                                            if let (NodeType::Client, NodeType::Client) = (
                                                self.network_view.own_node_info().node_type(),
                                                node_id.node_type(),
                                            ) {
                                                continue;
                                            }

                                            warn!("{:?} // Connecting to node {:?} as we don't know it yet", self.network_view.node_id(), node_id);
                                            let _ = network_node
                                                .connections()
                                                .connect_to_node(node_id.node_id());
                                        }
                                    }

                                    certificates.push((header.from(), signature));

                                    if certificates.len() > (*contacted * 2 / 3) {
                                        let known_nodes = self
                                            .network_view
                                            .known_nodes()
                                            .into_iter()
                                            .filter(|node| *node != self.network_view.node_id())
                                            .collect::<Vec<_>>();

                                        warn!("We have enough certificates to join the network {}, moving to introduction phase. Broadcasting Hello Request to {:?}", certificates.len(), known_nodes);

                                        let hello_request =
                                            NetworkReconfigMessageType::NetworkHelloRequest(
                                                self.network_view.node_triple(),
                                                certificates.clone(),
                                            );

                                        let introduction_message =
                                            ReconfigurationMessage::NetworkReconfig(
                                                NetworkReconfigMessage::new(
                                                    seq_gen.next_seq(),
                                                    hello_request,
                                                ),
                                            );

                                        let _ = network_node.outgoing_stub().broadcast_signed(
                                            introduction_message,
                                            known_nodes.into_iter(),
                                        );

                                        self.move_to_intro_phase();
                                    }
                                }
                                NetworkJoinResponseMessage::Rejected(rejection_reason) => {
                                    error!(
                                        "We were rejected from the network: {:?} by the node {:?}",
                                        rejection_reason,
                                        header.from()
                                    );
                                }
                            }
                        } else {
                            warn!("Received a network join response from {:?} but we had already seen it", header.from());
                        }

                        NetworkProtocolResponse::Nil
                    }
                    NetworkReconfigMessageType::NetworkHelloRequest(
                        hello_request,
                        confirmations,
                    ) => {
                        self.handle_hello_request(
                            network_node,
                            reconf_msg_handler,
                            header,
                            seq,
                            hello_request,
                            confirmations,
                        );

                        NetworkProtocolResponse::Nil
                    }
                    NetworkReconfigMessageType::NetworkHelloReply(_known_nodes) => {
                        // Ignored as we are not yet in this phase
                        NetworkProtocolResponse::Nil
                    }
                }
            }
            NetworkNodeState::IntroductionPhase {
                contacted,
                responded,
            } => {
                match message {
                    NetworkReconfigMessageType::NetworkJoinRequest(join_request) => {
                        return self.handle_join_request(
                            network_node,
                            reconf_msg_handler,
                            header,
                            seq,
                            join_request,
                        );
                    }
                    NetworkReconfigMessageType::NetworkJoinResponse(_) => {
                        // Ignored, we are already a stable member of the network
                    }
                    NetworkReconfigMessageType::NetworkHelloRequest(sender_info, confirmations) => {
                        self.handle_hello_request(
                            network_node,
                            reconf_msg_handler,
                            header,
                            seq,
                            sender_info,
                            confirmations,
                        )
                    }
                    NetworkReconfigMessageType::NetworkHelloReply(known_nodes) => {
                        if responded.insert(header.from()) {
                            let unknown_nodes =
                                self.network_view.handle_received_network_view(known_nodes);

                            for node_id in unknown_nodes {
                                if !network_node
                                    .connections()
                                    .has_connection(&node_id.node_id())
                                {
                                    if let (NodeType::Client, NodeType::Client) = (
                                        self.network_view.own_node_info().node_type(),
                                        node_id.node_type(),
                                    ) {
                                        continue;
                                    }

                                    warn!(
                                        "{:?} // Connecting to node {:?} as we don't know it yet",
                                        self.network_view.node_id(),
                                        node_id
                                    );

                                    let _ = network_node
                                        .connections()
                                        .connect_to_node(node_id.node_id());
                                }
                            }
                        }

                        if responded.len() > (*contacted * 2 / 3) {
                            self.move_to_stable();

                            return NetworkProtocolResponse::Done;
                        }
                    }
                }

                NetworkProtocolResponse::Nil
            }
            NetworkNodeState::StableMember => {
                match message {
                    NetworkReconfigMessageType::NetworkJoinRequest(join_request) => {
                        return self.handle_join_request(
                            network_node,
                            reconf_msg_handler,
                            header,
                            seq,
                            join_request,
                        );
                    }
                    NetworkReconfigMessageType::NetworkJoinResponse(_) => {
                        // Ignored, we are already a stable member of the network
                    }
                    NetworkReconfigMessageType::NetworkHelloRequest(sender_info, confirmations) => {
                        self.handle_hello_request(
                            network_node,
                            reconf_msg_handler,
                            header,
                            seq,
                            sender_info,
                            confirmations,
                        );
                    }
                    NetworkReconfigMessageType::NetworkHelloReply(_) => {
                        // Ignored, we are already a stable member of the network
                    }
                }

                NetworkProtocolResponse::Nil
            }
            NetworkNodeState::LeavingNetwork => {
                // We are leaving the network, ignore all messages
                NetworkProtocolResponse::Nil
            }
            NetworkNodeState::Init => NetworkProtocolResponse::Nil,
        }
    }

    pub(super) fn handle_hello_request<NT>(
        &self,
        network_node: &Arc<NT>,
        reconf_msg_handler: &ReconfigurationNetworkCommunication,
        header: Header,
        seq: SeqNo,
        node: NodeInfo,
        confirmations: Vec<NetworkJoinCert>,
    ) where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        match self
            .network_view
            .is_valid_network_hello(node.clone(), confirmations)
        {
            Ok(_) => {
                info!("Received a node hello message from node {:?} with enough certificates. Adding it to our known nodes", node);

                let known_nodes = {
                    let read_guard = self.network_view.known_nodes.read().unwrap();

                    KnownNodesMessage::from(&*read_guard)
                };

                let hello_reply_message = NetworkReconfigMessage::new(
                    seq,
                    NetworkReconfigMessageType::NetworkHelloReply(known_nodes),
                );

                let _ = network_node.outgoing_stub().send_signed(
                    ReconfigurationMessage::NetworkReconfig(hello_reply_message),
                    header.from(),
                    true,
                );

                if self.network_view.handle_node_introduced(node.clone()) {
                    info!("Node {:?} has joined the network and we hadn't seen it before, sending network update to the network layer", node.node_id());
                } else {
                    info!(
                        "Node {:?} has joined the network but we had already seen it before",
                        node.node_id()
                    );
                }

                let public_key = self.network_view.get_pk_for_node(&node.node_id()).unwrap();

                let connection_permitted =
                    ReconfigurationNetworkUpdateMessage::NodeConnectionPermitted(
                        node.node_id(),
                        node.node_type(),
                        public_key,
                    );

                let _ = reconf_msg_handler.send_reconfiguration_update(connection_permitted);
            }
            Err(err) => {
                error!(
                    "Received a network hello request from {:?} but it was not valid due to {:?}",
                    header.from(),
                    err
                );
            }
        }
    }

    pub(super) fn handle_join_request<NT>(
        &self,
        network_node: &Arc<NT>,
        reconf_msg_handler: &ReconfigurationNetworkCommunication,
        header: Header,
        seq: SeqNo,
        node: NodeInfo,
    ) -> NetworkProtocolResponse
    where
        NT: RegularNetworkStub<ReconfData> + 'static,
    {
        let network = network_node.clone();

        let reconf_msg_handler = reconf_msg_handler.clone();

        let target = header.from();

        let network_view = self.network_view.clone();

        let network_update = self.network_update.clone();

        threadpool::execute(move || {
            let triple = node;

            let result = rt::block_on(network_view.can_introduce_node(triple.clone()));

            let message = NetworkReconfigMessage::new(
                seq,
                NetworkReconfigMessageType::NetworkJoinResponse(result),
            );

            let reconfig_message = ReconfigurationMessage::NetworkReconfig(message);

            info!("Responding to network join request a network join response to {:?} with message {:?}", target, reconfig_message);

            let _ = network
                .outgoing_stub()
                .send_signed(reconfig_message, target, true);

            let _ = network_update.send(NodeConnectionUpdateMessage::NodeConnected(triple.clone()));

            if network_view.handle_node_introduced(triple.clone()) {
                info!("Node {:?} has joined the network and we hadn't seen it before, sending network update to the network layer", triple.node_id());

                let public_key = network_view.get_pk_for_node(&triple.node_id()).unwrap();

                let connection_permitted =
                    ReconfigurationNetworkUpdateMessage::NodeConnectionPermitted(
                        triple.node_id(),
                        triple.node_type(),
                        public_key,
                    );

                let _ = reconf_msg_handler.send_reconfiguration_update(connection_permitted);
            } else {
                info!(
                    "Node {:?} has joined the network but we had already seen it before",
                    triple.node_id()
                );
            }
        });

        NetworkProtocolResponse::Nil
    }

    fn move_to_joining(&mut self, contacted: usize) {
        self.current_state = NetworkNodeState::JoiningNetwork {
            contacted,
            responded: Default::default(),
            certificates: Default::default(),
        };
    }

    fn move_to_intro_phase(&mut self) {
        self.current_state = NetworkNodeState::IntroductionPhase {
            contacted: 0,
            responded: Default::default(),
        };
    }

    fn move_to_stable(&mut self) {
        self.current_state = NetworkNodeState::StableMember;
    }

    pub fn new(
        network_view: Arc<NetworkInfo>,
        current_state: NetworkNodeState,
        network_update: ChannelSyncTx<NodeConnectionUpdateMessage>,
    ) -> Self {
        Self {
            network_view,
            current_state,
            network_update,
        }
    }
}

#[derive(Error, Debug)]
pub enum ValidNetworkHelloError {
    #[error("One or more signatures is invalid.")]
    InvalidSignatures,
    #[error("Non boostrap certificate")]
    NonBootstrapCertificate,
    #[error("Not enough certificates {required:?} vs provided {provided:?}")]
    NotEnoughCertificates { required: usize, provided: usize },
}
