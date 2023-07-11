use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use futures::future::join_all;
use log::debug;
use atlas_common::channel::OneShotRx;
use atlas_common::crypto::signature::{KeyPair, PublicKey};
use atlas_common::node_id::NodeId;
use atlas_common::peer_addr::PeerAddr;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use crate::config::ReconfigurableNetworkConfig;
use crate::message::{KnownNodesMessage, NetworkJoinRejectionReason, NetworkJoinResponseMessage, NodeTriple};

/// The reconfiguration module.
/// Provides various utilities for allowing reconfiguration of the network
/// Such as message definitions, important types and etc.
///
/// This module will then be used by the parts of the system which must be reconfigurable
/// (For example, the network

pub type NetworkPredicate =
fn(Arc<NetworkInfo>, NodeTriple) -> OneShotRx<Option<NetworkJoinRejectionReason>>;


/// Our current view of the network and the information about our own node
/// This is the node data for the network information. This does not
/// directly store information about the quorum, only about the nodes that we
/// currently know about
pub struct NetworkInfo {
    node_id: NodeId,
    key_pair: Arc<KeyPair>,

    address: PeerAddr,

    // The list of nodes that we currently know in the network
    known_nodes: RwLock<KnownNodes>,

    /// Predicates that must be satisfied for a node to be allowed to join the network
    predicates: Vec<NetworkPredicate>,
}


impl NetworkInfo {
    pub fn init_from_config(config: ReconfigurableNetworkConfig) -> Self {
        let ReconfigurableNetworkConfig {
            node_id,
            key_pair,
            our_address,
            known_nodes,
        } = config;

        NetworkInfo {
            node_id,
            key_pair: Arc::new(key_pair),
            address: our_address,
            known_nodes: RwLock::new(KnownNodes::from_known_list(known_nodes)),
            predicates: Vec::new(),
        }
    }

    pub fn empty_network_node(
        node_id: NodeId,
        key_pair: KeyPair,
        address: PeerAddr,
    ) -> Self {
        NetworkInfo {
            node_id,
            key_pair: Arc::new(key_pair),
            known_nodes: RwLock::new(KnownNodes::empty()),
            predicates: vec![],
            address,
        }
    }

    /// Initialize a NetworkNode with a list of already known Nodes so we can bootstrap our information
    /// From them.
    pub fn with_bootstrap_nodes(
        node_id: NodeId,
        key_pair: KeyPair,
        address: PeerAddr,
        bootstrap_nodes: BTreeMap<NodeId, (PeerAddr, Vec<u8>)>,
    ) -> Self {
        let node = NetworkInfo::empty_network_node(node_id, key_pair, address);

        {
            let mut write_guard = node.known_nodes.write().unwrap();

            for (node_id, (addr, pk_bytes)) in bootstrap_nodes {
                let public_key = PublicKey::from_bytes(&pk_bytes[..]).unwrap();

                write_guard.node_keys.insert(node_id, public_key);
                write_guard.node_addrs.insert(node_id, addr);
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
    pub(crate) fn handle_node_introduced(&self, node: NodeTriple) {

        debug!("Received a node introduction message from node {:?}. Handling it", node);

        let mut write_guard = self.known_nodes.write().unwrap();

        Self::handle_single_node_introduced(&mut *write_guard, node)
    }

    /// Handle us having received a successfull network join response, with the list of known nodes
    pub(crate) fn handle_successfull_network_join(&self, known_nodes: KnownNodesMessage) {
        let mut write_guard = self.known_nodes.write().unwrap();

        debug!("Successfully joined the network. Updating our known nodes list with the received list {:?}", known_nodes);

        for node in known_nodes.into_nodes() {
            Self::handle_single_node_introduced(&mut *write_guard, node);
        }
    }

    fn handle_single_node_introduced(write_guard: &mut KnownNodes, node: NodeTriple) {
        let node_id = node.node_id();

        if !write_guard.node_keys.contains_key(&node_id) {
            let public_key = PublicKey::from_bytes(&node.public_key()[..]).unwrap();

            write_guard.node_keys.insert(node_id, public_key);
            write_guard.node_addrs.insert(node_id, node.addr().clone());
        }
    }

    /// Can we introduce this node to the network
    pub async fn can_introduce_node(
        self: Arc<Self>,
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

        self.handle_node_introduced(node_id);

        let read_guard = self.known_nodes.read().unwrap();

        return NetworkJoinResponseMessage::Successful(KnownNodesMessage::from(&*read_guard));
    }

    pub fn get_pk_for_node(&self, node: &NodeId) -> Option<PublicKey> {
        self.known_nodes
            .read()
            .unwrap()
            .node_keys
            .get(node)
            .cloned()
    }

    pub fn get_addr_for_node(&self, node: &NodeId) -> Option<PeerAddr> {
        self.known_nodes
            .read()
            .unwrap()
            .node_addrs
            .get(node)
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
            .node_addrs
            .keys()
            .cloned()
            .collect()
    }

    pub fn node_triple(&self) -> NodeTriple {
        NodeTriple::new(
            self.node_id,
            self.key_pair.public_key_bytes().to_vec(),
            self.address.clone(),
        )
    }
}

impl NetworkInformationProvider for NetworkInfo {
    fn get_own_addr(&self) -> PeerAddr {
        self.address.clone()
    }

    fn get_key_pair(&self) -> &Arc<KeyPair> {
        &self.key_pair
    }

    fn get_public_key(&self, node: &NodeId) -> Option<PublicKey> {
        self.known_nodes.read().unwrap().node_keys.get(node).cloned()
    }

    fn get_addr_for_node(&self, node: &NodeId) -> Option<PeerAddr> {
        self.known_nodes.read().unwrap().node_addrs.get(node).cloned()
    }
}

/// The map of known nodes in the network, independently of whether they are part of the current
/// quorum or not
#[derive(Clone)]
pub struct KnownNodes {
    pub(crate) node_keys: BTreeMap<NodeId, PublicKey>,
    pub(crate) node_addrs: BTreeMap<NodeId, PeerAddr>,
}


impl KnownNodes {
    fn empty() -> Self {
        Self {
            node_keys: BTreeMap::new(),
            node_addrs: BTreeMap::new(),
        }
    }

    fn from_known_list(nodes: Vec<NodeTriple>) -> Self {
        let mut known_nodes = Self::empty();

        for node in nodes {
            NetworkInfo::handle_single_node_introduced(&mut known_nodes, node)
        }

        known_nodes
    }

    pub fn node_keys(&self) -> &BTreeMap<NodeId, PublicKey> {
        &self.node_keys
    }

    pub fn node_addrs(&self) -> &BTreeMap<NodeId, PeerAddr> {
        &self.node_addrs
    }
}