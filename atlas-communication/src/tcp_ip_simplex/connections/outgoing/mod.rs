use std::sync::Arc;
use atlas_common::socket::{SecureSocket};
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::Serializable;
use crate::tcp_ip_simplex::connections::{PeerConnection, SimplexConnections};
use crate::tcp_ip_simplex::connections::ping_handler::PingHandler;
use crate::tcpip::connections::ConnHandle;

mod asynchronous;


pub(super) fn spawn_outgoing_task_handler<NI, RM, PM>(
    conn_handle: ConnHandle,
    node_conns: Arc<SimplexConnections<NI, RM, PM>>,
    connection: Arc<PeerConnection<RM, PM>>,
    ping: Arc<PingHandler>,
    socket: SecureSocket)
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    let rx = ping.register_ping_channel(connection.peer_node_id, conn_handle.id());

    match socket {
        SecureSocket::Async(asynchronous) => {
            asynchronous::spawn_outgoing_task(conn_handle, ping, rx, node_conns, connection, asynchronous);
        }
        SecureSocket::Sync(synchronous) => {
            //synchronous::spawn_outgoing_thread(conn_handle, connection, synchronous);
        }
    }
}