use std::sync::Arc;
use atlas_common::socket::{SecureReadHalf};
use crate::reconfiguration_node::{NetworkInformationProvider};
use crate::serialize::Serializable;

use crate::tcpip::connections::{ConnHandle, PeerConnection, PeerConnections};

pub mod asynchronous;
pub mod synchronous;

pub(super) fn spawn_incoming_task_handler<NI, RM, PM>(
    conn_handle: ConnHandle,
    node_conns: Arc<PeerConnections<NI, RM, PM>>,
    connected_peer: Arc<PeerConnection<RM, PM>>,
    socket: SecureReadHalf)
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static
{
    match socket {
        SecureReadHalf::Async(asynchronous) => {
            asynchronous::spawn_incoming_task(conn_handle, node_conns, connected_peer, asynchronous);
        }
        SecureReadHalf::Sync(synchronous) => {
            synchronous::spawn_incoming_thread(conn_handle, node_conns, connected_peer, synchronous);
        }
    }
}