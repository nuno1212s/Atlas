use std::sync::Arc;
use log::error;

use atlas_common::socket::SecureWriteHalfSync;
use atlas_metrics::metrics::metric_duration;
use crate::metric::COMM_REQUEST_SEND_TIME_ID;
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::Serializable;

use crate::tcpip::connections::{ConnHandle, PeerConnection, PeerConnections};

pub(super) fn spawn_outgoing_thread<NI, RM, PM>(
    conn_handle: ConnHandle,
    node_conns: Arc<PeerConnections<NI, RM, PM>>,
    mut peer: Arc<PeerConnection<RM, PM>>,
    mut socket: SecureWriteHalfSync)
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    std::thread::Builder::new()
        .name(format!("Outgoing connection thread"))
        .spawn(move || {
            let rx = peer.to_send_handle().clone();

            loop {
                let (to_send, callback, dispatch_time, flush, send_rq_time) = match rx.recv() {
                    Ok(message) => { message }
                    Err(error_kind) => {
                        error!("Failed to receive message to send. {:?}", error_kind);

                        break;
                    }
                };

                if conn_handle.is_cancelled() {
                    peer.peer_message(to_send, callback, flush, send_rq_time).unwrap();

                    return;
                }

                match to_send.write_to_sync(&mut socket, true) {
                    Ok(_) => {
                        metric_duration(COMM_REQUEST_SEND_TIME_ID, dispatch_time.elapsed());

                        if let Some(callback) = callback {
                            callback(true);
                        }
                    }
                    Err(error_kind) => {
                        error!("Failed to write message to socket. {:?}", error_kind);

                        peer.peer_message(to_send, callback, flush, send_rq_time).unwrap();

                        break;
                    }
                }
            }

            let remaining_conns = peer.delete_connection(conn_handle.id());

            node_conns.handle_conn_lost(&peer.peer_node_id, remaining_conns);
        }).unwrap();
}