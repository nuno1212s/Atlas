mod asynchronous;

use std::sync::Arc;
use atlas_common::socket::SecureSocket;
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::Serializable;
use crate::tcp_ip_simplex::connections::{PeerConnection, SimplexConnections};
use crate::tcpip::connections::ConnHandle;

pub(super) fn spawn_incoming_task_handler<NI, RM, PM>(
    conn_handle: ConnHandle,
    node_conns: Arc<SimplexConnections<NI, RM, PM>>,
    connected_peer: Arc<PeerConnection<RM, PM>>,
    socket: SecureSocket)
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    match socket {
        SecureSocket::Async(asynchronous) => {
            asynchronous::spawn_incoming_task(conn_handle, node_conns, connected_peer, asynchronous);
        }
        SecureSocket::Sync(synchronous) => {
            todo!()
            //synchronous::spawn_incoming_thread(conn_handle, connected_peer, synchronous);
        }
    }
}