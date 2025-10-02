use std::net::SocketAddr;

#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

///Represents the server addresses of a peer
///Clients will only have 1 address while replicas will have 2 addresses (1 for facing clients,
/// 1 for facing replicas)
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct PeerAddr {
    // All nodes have a replica facing socket: (SocketAddr, String),
    socket: SocketAddr,
    hostname: String,
}

impl PeerAddr {
    pub fn new(socket: SocketAddr, hostname: String) -> Self {
        Self { socket, hostname }
    }

    pub fn socket(&self) -> &SocketAddr {
        &self.socket
    }

    pub fn hostname(&self) -> &String {
        &self.hostname
    }

    pub fn into_inner(self) -> (SocketAddr, String) {
        (self.socket, self.hostname)
    }
}
