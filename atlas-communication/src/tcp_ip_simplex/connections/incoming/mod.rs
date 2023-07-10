mod asynchronous;

use std::sync::Arc;
use atlas_common::socket::SecureSocket;
use crate::serialize::Serializable;
use crate::tcp_ip_simplex::connections::PeerConnection;
use crate::tcpip::connections::ConnHandle;

pub(super) fn spawn_incoming_task_handler<RM, PM>(
    conn_handle: ConnHandle,
    connected_peer: Arc<PeerConnection<RM, PM>>,
    socket: SecureSocket)
    where RM: Serializable + 'static, PM: Serializable + 'static {
    match socket {
        SecureSocket::Async(asynchronous) => {
            asynchronous::spawn_incoming_task(conn_handle, connected_peer, asynchronous);
        }
        SecureSocket::Sync(synchronous) => {
            todo!()
            //synchronous::spawn_incoming_thread(conn_handle, connected_peer, synchronous);
        }
    }
}