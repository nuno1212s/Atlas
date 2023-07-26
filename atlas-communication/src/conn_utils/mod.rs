use atlas_common::node_id::{NodeId, NodeType};
use crate::config::TcpConfig;
use crate::reconfiguration_node::NetworkInformationProvider;

pub type Callback = Option<Box<dyn FnOnce(bool) -> () + Send>>;

/// The amount of parallel TCP connections we should try to maintain for
/// each connection
#[derive(Clone)]
pub struct ConnCounts {
    replica_connections: usize,
    client_connections: usize,
}

impl ConnCounts {
    pub(crate) fn from_tcp_config(tcp: &TcpConfig) -> Self {
        Self {
            replica_connections: tcp.replica_concurrent_connections,
            client_connections: tcp.client_concurrent_connections,
        }
    }

    /// How many connections should we maintain with a given node
    pub(crate) fn get_connections_to_node<NI>(&self, my_id: NodeId, other_id: NodeId, info_provider: &NI) -> usize
        where NI: NetworkInformationProvider {
        let node_type = info_provider.get_own_node_type();

        let other_node_type = info_provider.get_node_type(&other_id).unwrap();

        return match (node_type, other_node_type) {
            (NodeType::Replica, NodeType::Replica) => self.replica_connections,
            _ => self.client_connections
        };
    }
}