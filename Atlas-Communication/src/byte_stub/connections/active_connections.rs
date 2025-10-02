use crate::byte_stub::peer_conn::PeerConnection;
use crate::serialization::Serializable;
use atlas_common::node_id::NodeId;
use dashmap::DashMap;
use getset::Getters;

pub type ActiveCNNMap<CN, R, O, S, A, L> = DashMap<NodeId, PeerConnection<CN, R, O, S, A, L>>;

/// The active stubs, connecting to a given peer
#[derive(Getters)]
pub struct ActiveConnections<CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    id: NodeId,
    #[get = "pub"]
    loopback: PeerConnection<CN, R, O, S, A, L>,
    connection_map: ActiveCNNMap<CN, R, O, S, A, L>,
}

impl<CN, R, O, S, A, L> ActiveConnections<CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    CN: Clone,
    L: Clone,
{
    pub fn has_connection(&self, node: &NodeId) -> bool {
        self.connection_map.contains_key(node)
    }

    pub fn get_connection(&self, node: &NodeId) -> Option<PeerConnection<CN, R, O, S, A, L>> {
        if *node == self.id {
            return Some(self.loopback.clone());
        }

        self.connection_map.get(node).map(|map| map.value().clone())
    }

    pub fn add_connection(&self, node: NodeId, connection: PeerConnection<CN, R, O, S, A, L>) {
        self.connection_map.insert(node, connection);
    }

    pub fn remove_connection(&self, node: &NodeId) -> Option<PeerConnection<CN, R, O, S, A, L>> {
        self.connection_map
            .remove(node)
            .map(|(_node_id, conn)| conn)
    }
}

impl<CN, R, O, S, A, L> ActiveConnections<CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub(crate) fn init(id: NodeId, loopback: PeerConnection<CN, R, O, S, A, L>) -> Self {
        Self {
            id,
            loopback,
            connection_map: Default::default(),
        }
    }
}
