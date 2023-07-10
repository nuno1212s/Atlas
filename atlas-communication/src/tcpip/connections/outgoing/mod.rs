use std::sync::Arc;
use atlas_common::socket::{SecureWriteHalf};
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::Serializable;

use crate::tcpip::connections::{ConnHandle, PeerConnection, PeerConnections};

pub mod asynchronous;
pub mod synchronous;

pub(super) fn spawn_outgoing_task_handler<NI, RM, PM>(
    conn_handle: ConnHandle,
    node_conns: Arc<PeerConnections<NI, RM, PM>>,
    connection: Arc<PeerConnection<RM, PM>>,
    socket: SecureWriteHalf)
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    match socket {
        SecureWriteHalf::Async(asynchronous) => {
            asynchronous::spawn_outgoing_task(conn_handle, node_conns, connection, asynchronous);
        }
        SecureWriteHalf::Sync(synchronous) => {
            synchronous::spawn_outgoing_thread(conn_handle, node_conns, connection, synchronous);
        }
    }
}