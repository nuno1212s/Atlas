use std::net::SocketAddr;

#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};


///Represents the server addresses of a peer
///Clients will only have 1 address while replicas will have 2 addresses (1 for facing clients,
/// 1 for facing replicas)
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct PeerAddr {
    // All nodes have a replica facing socket
    pub replica_facing_socket: (SocketAddr, String),
    // Only replicas have a client facing socket
    pub client_facing_socket: Option<(SocketAddr, String)>,
}

impl PeerAddr {
    pub fn new(client_addr: (SocketAddr, String)) -> Self {
        Self {
            replica_facing_socket: client_addr,
            client_facing_socket: None,
        }
    }

    pub fn new_replica(
        client_addr: (SocketAddr, String),
        replica_addr: (SocketAddr, String),
    ) -> Self {
        Self {
            replica_facing_socket: client_addr,
            client_facing_socket: Some(replica_addr),
        }
    }
}