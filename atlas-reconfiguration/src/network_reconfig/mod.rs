use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use futures::future::join_all;
use futures::SinkExt;
use log::{debug, error, info, warn};

use atlas_common::async_runtime as rt;
use atlas_common::channel::OneShotRx;
use atlas_common::crypto::signature;
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::ordering::SeqNo;
use atlas_common::peer_addr::PeerAddr;
use atlas_communication::message::Header;
use atlas_communication::NodeConnections;
use atlas_communication::reconfiguration_node::{NetworkInformationProvider, NetworkUpdateMessage, ReconfigurationNetworkUpdate, ReconfigurationNode};
use atlas_core::timeouts::Timeouts;

use crate::config::ReconfigurableNetworkConfig;
use crate::message::{KnownNodesMessage, NetworkJoinCert, NetworkJoinRejectionReason, NetworkJoinResponseMessage, NetworkReconfigMessage, NodeTriple, QuorumReconfigMessage, ReconfData, ReconfigurationMessage, ReconfigurationMessageType, signatures};
use crate::{NetworkProtocolResponse, SeqNoGen, TIMEOUT_DUR};

/// The reconfiguration module.
/// Provides various utilities for allowing reconfiguration of the network
/// Such as message definitions, important types and etc.
///
/// This module will then be used by the parts of the system which must be reconfigurable
/// (For example, the network

pub type NetworkPredicate = fn(Arc<NetworkInfo>, NodeTriple) -> OneShotRx<Option<NetworkJoinRejectionReason>>;


/// Our current view of the network and the information about our own node
/// This is the node data for the network information. This does not
/// directly store information about the quorum, only about the nodes that we
/// currently know about
pub struct NetworkInfo {
    node_id: NodeId,
    node_type: NodeType,
    key_pair: Arc<KeyPair>,

    address: PeerAddr,

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
            node_id,
            node_type,
            key_pair: Arc::new(key_pair),
            address: our_address,
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
            node_id,
            node_type,
            key_pair: Arc::new(key_pair),
            known_nodes: RwLock::new(KnownNodes::empty()),
            predicates: vec![],
            address,
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

                let info = NodeInfo {
                    node_type,
                    pk: public_key,
                    addr,
                };

                write_guard.node_info.insert(node_id, info);
            }
        }

        node
    }

    pub fn register_join_predicate(&mut self, predicate: NetworkPredicate) {
        self.predicates.push(predicate)
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Handle a node having introduced itself to us by inserting it into our known nodes
    pub(crate) fn handle_node_introduced(&self, node: NodeTriple) -> bool {
        debug!("Handling a node having been introduced {:?}. Handling it", node);

        let mut write_guard = self.known_nodes.write().unwrap();

        Self::handle_single_node_introduced(&mut *write_guard, node)
    }

    pub(crate) fn is_valid_network_hello(&self, node: NodeTriple, certificates: Vec<NetworkJoinCert>) -> bool {
        debug!("Received a node hello message from node {:?}. Handling it", node);

        for (from, signature) in &certificates {
            let from_pk = self.get_pk_for_node(from);
            if let Some(pk) = from_pk {
                if !signatures::verify_node_triple_signature(&node, signature, &pk) {
                    error!("Received a node hello message from node {:?} with invalid signature. Ignoring it",node);
                    return false;
                }
            } else {
                error!("Received a node hello message from node {:?} with certificate from node {:?} which we don't know. Ignoring it",node, from);
            }

            if !self.bootstrap_nodes.contains(&from) {
                error!("Received a node hello message from node {:?} with certificate from node {:?} which is not a bootstrap node. Ignoring it",node, from);
                return false;
            }
        }

        let mut required = (self.bootstrap_nodes.len() * 2 / 3) + 1;

        if self.bootstrap_nodes.contains(&node.node_id()) {
            // If the node is a bootstrap node, then we don't need one of the nodes
            required -= 1;
        }

        if certificates.len() < required {
            error!("Received a node hello message from node {:?} with less certificates than 2n/3 bootstrap nodes {:?} vs required {:?}. Ignoring it", node, certificates.len(), required);
            return false;
        }


        return true;
    }

    /// Handle us having received a successfull network join response, with the list of known nodes
    /// Returns a list of nodes that we didn't know before, but that have now been added
    pub(crate) fn handle_received_network_view(&self, known_nodes: KnownNodesMessage) -> Vec<NodeId> {
        let mut write_guard = self.known_nodes.write().unwrap();

        debug!("Updating our known nodes list with the received list {:?}", known_nodes);

        let mut new_nodes = Vec::with_capacity(known_nodes.known_nodes().len());

        for node in known_nodes.into_nodes() {
            let node_id = node.node_id();
            if Self::handle_single_node_introduced(&mut *write_guard, node) {
                new_nodes.push(node_id);
            }
        }

        new_nodes
    }

    fn handle_single_node_introduced(write_guard: &mut KnownNodes, node: NodeTriple) -> bool {
        let node_id = node.node_id();

        if !write_guard.node_info.contains_key(&node_id) {
            let public_key = PublicKey::from_bytes(&node.public_key()[..]).unwrap();

            let node_info = NodeInfo {
                node_type: node.node_type(),
                pk: public_key,
                addr: node.addr().clone(),
            };

            write_guard.node_info.insert(node_id, node_info);

            true
        } else {
            debug!("Node {:?} has already been introduced to us. Ignoring",node);

            false
        }
    }

    /// Can we introduce this node to the network
    pub async fn can_introduce_node(
        self: &Arc<Self>,
        node_id: NodeTriple,
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

        let signature = signatures::create_node_triple_signature(&node_id, &*self.key_pair).expect("Failed to sign node triple");

        let read_guard = self.known_nodes.read().unwrap();

        return NetworkJoinResponseMessage::Successful(signature, KnownNodesMessage::from(&*read_guard));
    }

    pub fn get_pk_for_node(&self, node: &NodeId) -> Option<PublicKey> {
        self.known_nodes
            .read()
            .unwrap().node_info()
            .get(node)
            .map(|info| info.pk())
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
        &self.address
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

    pub fn node_triple(&self) -> NodeTriple {
        NodeTriple::new(
            self.node_id,
            self.key_pair.public_key_bytes().to_vec(),
            self.address.clone(),
            self.node_type,
        )
    }

    pub fn bootstrap_nodes(&self) -> &Vec<NodeId> {
        &self.bootstrap_nodes
    }
}

impl NetworkInformationProvider for NetworkInfo {
    fn get_own_addr(&self) -> PeerAddr {
        self.address.clone()
    }

    fn get_key_pair(&self) -> &Arc<KeyPair> {
        &self.key_pair
    }

    fn get_own_node_type(&self) -> NodeType {
        self.node_type
    }

    fn get_node_type(&self, node: &NodeId) -> Option<NodeType> {
        self.known_nodes.read().unwrap().node_info.get(node).map(|info| info.node_type)
    }

    fn get_public_key(&self, node: &NodeId) -> Option<PublicKey> {
        self.known_nodes.read().unwrap().node_info.get(node).map(|info| info.pk.clone())
    }

    fn get_addr_for_node(&self, node: &NodeId) -> Option<PeerAddr> {
        self.known_nodes.read().unwrap().node_info.get(node).map(|info| info.addr.clone())
    }
}

/// The node info for a given network node
#[derive(Clone)]
pub(crate) struct NodeInfo {
    node_type: NodeType,
    pk: PublicKey,
    addr: PeerAddr,
}

impl NodeInfo {
    pub(crate) fn node_type(&self) -> NodeType {
        self.node_type
    }

    pub(crate) fn pk(&self) -> &PublicKey {
        &self.pk
    }

    pub(crate) fn addr(&self) -> &PeerAddr {
        &self.addr
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

    fn from_known_list(nodes: Vec<NodeTriple>) -> Self {
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
    JoiningNetwork { contacted: usize, responded: BTreeSet<NodeId>, certificates: Vec<NetworkJoinCert> },
    /// We are currently introducing ourselves to the network (and attempting to acquire all known nodes)
    IntroductionPhase { contacted: usize, responded: BTreeSet<NodeId> },
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
}

impl GeneralNodeInfo {
    /// Attempt to iterate and move our current state forward
    pub(super) fn iterate<NT>(&mut self, seq: &mut SeqNoGen, network_node: &Arc<NT>, timeouts: &Timeouts) -> NetworkProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            NetworkNodeState::Init => {
                let known_nodes: Vec<NodeId> = self.network_view.known_nodes().into_iter()
                    .filter(|node| *node != self.network_view.node_id()).collect();

                let join_message =
                    ReconfigurationMessage::new(seq.next_seq(),
                                                ReconfigurationMessageType::NetworkReconfig(
                                                    NetworkReconfigMessage::NetworkJoinRequest(self.network_view.node_triple())));

                let contacted = known_nodes.len();

                if known_nodes.is_empty() {
                    info!("No known nodes, joining network as a stable member");
                    self.current_state = NetworkNodeState::StableMember;
                }

                let mut node_results = Vec::new();

                for node in &known_nodes {
                    info!("{:?} // Connecting to node {:?}", self.network_view.node_id(), node);
                    let mut node_connection_results = network_node.node_connections().connect_to_node(*node);

                    node_results.push((*node, node_connection_results));
                }

                for (node, conn_results) in node_results {
                    for conn_result in conn_results {
                        if let Err(err) = conn_result.recv().unwrap() {
                            error!("Error while connecting to another node: {:?}", err);
                        }
                    }

                    info!("{:?} // Connected to node {:?}",self.network_view.node_id(), node);
                }

                info!("Broadcasting reconfiguration network join message to known nodes {:?}", known_nodes);

                network_node.broadcast_reconfig_message(join_message, known_nodes.into_iter()).unwrap();

                timeouts.timeout_reconfig_request(TIMEOUT_DUR, ((contacted * 2 / 3) + 1) as u32, seq.curr_seq());

                self.current_state = NetworkNodeState::JoiningNetwork {
                    contacted,
                    responded: Default::default(),
                    certificates: Default::default(),
                };

                return NetworkProtocolResponse::Running;
            }
            NetworkNodeState::IntroductionPhase { .. } => {}
            NetworkNodeState::JoiningNetwork { .. } => {}
            NetworkNodeState::StableMember => {}
            NetworkNodeState::LeavingNetwork => {}
        }

        NetworkProtocolResponse::Nil
    }

    pub(super) fn handle_timeout<NT>(&mut self, seq_gen: &mut SeqNoGen, network_node: &Arc<NT>, timeouts: &Timeouts) -> NetworkProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static
    {
        match &mut self.current_state {
            NetworkNodeState::JoiningNetwork { contacted, responded, certificates } => {
                info!("Joining network timeout triggered");

                let known_nodes: Vec<NodeId> = self.network_view.known_nodes().into_iter()
                    .filter(|node| *node != self.network_view.node_id()).collect();

                let contacted = known_nodes.len();

                if known_nodes.is_empty() {
                    info!("No known nodes, joining network as a stable member");
                    self.current_state = NetworkNodeState::StableMember;

                    return NetworkProtocolResponse::Done;
                }

                let mut node_results = Vec::new();

                for node in &known_nodes {
                    if network_node.node_connections().is_connected_to_node(node) {
                        continue;
                    }

                    info!("{:?} // Connecting to node {:?}", self.network_view.node_id(), node);

                    let mut node_connection_results = network_node.node_connections().connect_to_node(*node);

                    node_results.push((*node, node_connection_results));
                }

                for (node, conn_results) in node_results {
                    for conn_result in conn_results {
                        if let Err(err) = conn_result.recv().unwrap() {
                            error!("Error while connecting to another node: {:?}", err);
                        }
                    }
                }

                let join_message = ReconfigurationMessage::new(
                    seq_gen.next_seq(),
                    ReconfigurationMessageType::NetworkReconfig(NetworkReconfigMessage::NetworkJoinRequest(self.network_view.node_triple())),
                );

                info!("Received timeout, broadcasting reconfiguration network join message to known nodes {:?}", known_nodes);

                let _ = network_node.broadcast_reconfig_message(join_message, known_nodes.into_iter());

                timeouts.timeout_reconfig_request(TIMEOUT_DUR, (contacted / 2 + 1) as u32, seq_gen.curr_seq());

                self.current_state = NetworkNodeState::JoiningNetwork {
                    contacted,
                    responded: Default::default(),
                    certificates: Default::default(),
                };

                return NetworkProtocolResponse::Running;
            }
            NetworkNodeState::IntroductionPhase { .. } => {}
            NetworkNodeState::Init => {}
            NetworkNodeState::StableMember => {}
            NetworkNodeState::LeavingNetwork => {}
        }

        NetworkProtocolResponse::Nil
    }

    pub(super) fn is_response_to_request(&self, seq_gen: &SeqNoGen, header: &Header, seq: SeqNo, message: &NetworkReconfigMessage) -> bool {
        match message {
            NetworkReconfigMessage::NetworkJoinResponse(_) => true,
            NetworkReconfigMessage::NetworkHelloReply(_) => true,
            _ => false
        }
    }

    pub(super) fn handle_network_reconfig_msg<NT>(&mut self, seq_gen: &mut SeqNoGen, network_node: &Arc<NT>, timeouts: &Timeouts, header: Header, seq: SeqNo, message: NetworkReconfigMessage) -> NetworkProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        match &mut self.current_state {
            NetworkNodeState::JoiningNetwork { contacted, responded, certificates } => {
                // Avoid accepting double answers

                return match message {
                    NetworkReconfigMessage::NetworkJoinRequest(join_request) => {
                        info!("Received a network join request from {:?} while joining the network", header.from());

                        self.handle_join_request(network_node, header, seq, join_request)
                    }
                    NetworkReconfigMessage::NetworkJoinResponse(join_response) => {
                        if responded.insert(header.from()) {
                            match join_response {
                                NetworkJoinResponseMessage::Successful(signature, network_information) => {
                                    info!("We were accepted into the network by the node {:?}, current certificate count {:?}", header.from(), certificates.len());

                                    timeouts.cancel_reconfig_timeout(Some(seq));

                                    let unknown_nodes = self.network_view.handle_received_network_view(network_information);

                                    for node_id in unknown_nodes {
                                        if !network_node.node_connections().is_connected_to_node(&node_id) {
                                            warn!("{:?} // Connecting to node {:?} as we don't know it yet", self.network_view.node_id(), node_id);
                                            network_node.node_connections().connect_to_node(node_id);
                                        }
                                    }

                                    certificates.push((header.from(), signature));

                                    if certificates.len() >= (*contacted * 2 / 3) + 1 {
                                        warn!("We have enough certificates to join the network {}, moving to introduction phase", certificates.len());

                                        let introduction_message = ReconfigurationMessage::new(
                                            seq_gen.next_seq(),
                                            ReconfigurationMessageType::NetworkReconfig(NetworkReconfigMessage::NetworkHelloRequest(self.network_view.node_triple(), certificates.clone())),
                                        );

                                        let _ = network_node.broadcast_reconfig_message(introduction_message, self.network_view.known_nodes().into_iter());

                                        self.current_state = NetworkNodeState::IntroductionPhase {
                                            contacted: 0,
                                            responded: Default::default(),
                                        };
                                    }
                                }
                                NetworkJoinResponseMessage::Rejected(rejection_reason) => {
                                    error!("We were rejected from the network: {:?} by the node {:?}", rejection_reason, header.from());
                                }
                            }
                        } else {
                            warn!("Received a network join response from {:?} but we had already seen it", header.from());
                        }

                        NetworkProtocolResponse::Running
                    }
                    NetworkReconfigMessage::NetworkHelloRequest(hello_request, confirmations) => {
                        self.handle_hello_request(network_node, header, seq, hello_request, confirmations);

                        NetworkProtocolResponse::Running
                    }
                    NetworkReconfigMessage::NetworkHelloReply(known_nodes) => {
                        // Ignored as we are not yet in this phase
                        NetworkProtocolResponse::Running
                    }
                };
            }
            NetworkNodeState::IntroductionPhase {
                contacted, responded
            } => {
                match message {
                    NetworkReconfigMessage::NetworkJoinRequest(join_request) => {
                        return self.handle_join_request(network_node, header, seq, join_request);
                    }
                    NetworkReconfigMessage::NetworkJoinResponse(_) => {
                        // Ignored, we are already a stable member of the network
                    }
                    NetworkReconfigMessage::NetworkHelloRequest(sender_info, confirmations) => {
                        self.handle_hello_request(network_node, header, seq, sender_info, confirmations)
                    }
                    NetworkReconfigMessage::NetworkHelloReply(known_nodes) => {
                        if responded.insert(header.from()) {
                            let unknown_nodes = self.network_view.handle_received_network_view(known_nodes);

                            for node_id in unknown_nodes {
                                if !network_node.node_connections().is_connected_to_node(&node_id) {
                                    warn!("{:?} // Connecting to node {:?} as we don't know it yet", self.network_view.node_id(), node_id);

                                    network_node.node_connections().connect_to_node(node_id);
                                }
                            }
                        }

                        if responded.len() >= (*contacted * 2 / 3) + 1 {
                            self.current_state = NetworkNodeState::StableMember;

                            return NetworkProtocolResponse::Done;
                        }
                    }
                }

                return NetworkProtocolResponse::Nil;
            }
            NetworkNodeState::StableMember => {
                match message {
                    NetworkReconfigMessage::NetworkJoinRequest(join_request) => {
                        return self.handle_join_request(network_node, header, seq, join_request);
                    }
                    NetworkReconfigMessage::NetworkJoinResponse(_) => {
                        // Ignored, we are already a stable member of the network
                    }
                    NetworkReconfigMessage::NetworkHelloRequest(sender_info, confirmations) => {
                        self.handle_hello_request(network_node, header, seq, sender_info, confirmations);
                    }
                    NetworkReconfigMessage::NetworkHelloReply(_) => {
                        // Ignored, we are already a stable member of the network
                    }
                }

                return NetworkProtocolResponse::Nil;
            }
            NetworkNodeState::LeavingNetwork => {
                // We are leaving the network, ignore all messages
                return NetworkProtocolResponse::Nil;
            }
            NetworkNodeState::Init => {
                return NetworkProtocolResponse::Nil;
            }
        }
    }

    pub(super) fn handle_hello_request<NT>(&self, network_node: &Arc<NT>, header: Header, seq: SeqNo, node: NodeTriple, confirmations: Vec<NetworkJoinCert>)
        where NT: ReconfigurationNode<ReconfData> + 'static {
        if self.network_view.is_valid_network_hello(node.clone(), confirmations) {
            info!("Received a node hello message from node {:?} with enough certificates. Adding it to our known nodes", node);

            let known_nodes = {
                let read_guard = self.network_view.known_nodes.read().unwrap();

                KnownNodesMessage::from(&*read_guard)
            };

            let hello_reply = ReconfigurationMessageType::NetworkReconfig(NetworkReconfigMessage::NetworkHelloReply(known_nodes));

            network_node.send_reconfig_message(ReconfigurationMessage::new(seq, hello_reply), header.from());

            if self.network_view.handle_node_introduced(node.clone()) {
                info!("Node {:?} has joined the network and we hadn't seen it before, sending network update to the network layer", node.node_id());

                let public_key = self.network_view.get_pk_for_node(&node.node_id()).unwrap();

                let connection_permitted = NetworkUpdateMessage::NodeConnectionPermitted(node.node_id(), node.node_type(), public_key);

                network_node.reconfiguration_network_update().send_reconfiguration_update(connection_permitted);
            }
        } else {
            error!("Received a network hello request from {:?} but it was not valid", header.from());
        }
    }

    pub(super) fn handle_join_request<NT>(&self, network_node: &Arc<NT>, header: Header, seq: SeqNo, node: NodeTriple) -> NetworkProtocolResponse
        where NT: ReconfigurationNode<ReconfData> + 'static {
        let network = network_node.clone();

        let target = header.from();

        let network_view = self.network_view.clone();

        rt::spawn(async move {
            let triple = node;

            let result = network_view.can_introduce_node(triple.clone()).await;

            let message = NetworkReconfigMessage::NetworkJoinResponse(result);

            let reconfig_message = ReconfigurationMessage::new(seq, ReconfigurationMessageType::NetworkReconfig(message));

            let _ = network.send_reconfig_message(reconfig_message, target);

            if network_view.handle_node_introduced(triple.clone()) {
                info!("Node {:?} has joined the network and we hadn't seen it before, sending network update to the network layer", triple.node_id());

                let public_key = network_view.get_pk_for_node(&triple.node_id()).unwrap();

                let connection_permitted = NetworkUpdateMessage::NodeConnectionPermitted(triple.node_id(), triple.node_type(), public_key);

                network.reconfiguration_network_update().send_reconfiguration_update(connection_permitted);
            }
        });

        NetworkProtocolResponse::Nil
    }

    pub fn new(network_view: Arc<NetworkInfo>, current_state: NetworkNodeState) -> Self {
        Self { network_view, current_state }
    }
}