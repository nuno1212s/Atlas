use std::sync::Arc;
use atlas_common::channel::ChannelMixedRx;
use atlas_common::socket::{SecureWriteHalf, SecureWriteHalfSync};
use crate::serialize::Serializable;

use crate::tcpip::connections::{ConnHandle, PeerConnection, NetworkSerializedMessage};

pub mod asynchronous;
pub mod synchronous;

pub(super) fn spawn_outgoing_task_handler<RM, PM>(
    conn_handle: ConnHandle,
    connection: Arc<PeerConnection<RM, PM>>,
    socket: SecureWriteHalf)
    where RM: Serializable + 'static, PM: Serializable + 'static {
    match socket {
        SecureWriteHalf::Async(asynchronous) => {
            asynchronous::spawn_outgoing_task(conn_handle, connection, asynchronous);
        }
        SecureWriteHalf::Sync(synchronous) => {
            synchronous::spawn_outgoing_thread(conn_handle, connection, synchronous);
        }
    }
}