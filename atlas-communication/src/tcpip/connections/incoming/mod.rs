use std::sync::Arc;
use atlas_common::socket::{SecureReadHalf};

use crate::tcpip::connections::{ConnHandle, PeerConnection};

pub mod asynchronous;
pub mod synchronous;

pub(super) fn spawn_incoming_task_handler<RM, PM>(
    conn_handle: ConnHandle,
    connected_peer: Arc<PeerConnection<RM, PM>>,
    socket: SecureReadHalf) {

    match socket {
        SecureReadHalf::Async(asynchronous) => {
            asynchronous::spawn_incoming_task(conn_handle, connected_peer, asynchronous);
        }
        SecureReadHalf::Sync(synchronous) => {
            synchronous::spawn_incoming_thread(conn_handle, connected_peer, synchronous);
        }
    }
}