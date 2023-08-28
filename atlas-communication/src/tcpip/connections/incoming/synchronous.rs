use std::io::Read;
use std::sync::Arc;
use bytes::BytesMut;
use log::error;
use atlas_common::socket::{SecureReadHalfSync};
use crate::cpu_workers;
use crate::message::{Header, NetworkMessage, NetworkMessageKind, StoredMessage};
use crate::reconfiguration_node::{NetworkInformationProvider, ReconfigurationMessageHandler};
use crate::serialize::Serializable;
use crate::tcpip::connections::{ConnHandle, PeerConnection, PeerConnections};

pub(super) fn spawn_incoming_thread<NI, RM, PM>(
    conn_handle: ConnHandle,
    node_conns: Arc<PeerConnections<NI, RM, PM>>,
    peer: Arc<PeerConnection<RM, PM>>,
    mut socket: SecureReadHalfSync)
    where
        NI: NetworkInformationProvider + 'static,
        RM: Serializable + 'static,
        PM: Serializable + 'static {
    std::thread::Builder::new()
        .spawn(move || {
            let mut read_buffer = BytesMut::with_capacity(Header::LENGTH);
            let client_pool_rq = Arc::clone(peer.client_pool_peer());
            let peer_id = peer.client_pool_peer().client_id().clone();

            loop {
                read_buffer.resize(Header::LENGTH, 0);

                if let Err(err) = socket.read_exact(&mut read_buffer[..Header::LENGTH]) {
                    // errors reading -> faulty connection;
                    // drop this socket
                    error!("Failed to read header from socket, faulty connection {:?}", err);
                    break;
                }

                // we are passing the correct length, safe to use unwrap()
                let header = Header::deserialize_from(&read_buffer[..Header::LENGTH]).unwrap();

                // reserve space for message
                //
                //FIXME: add a max bound on the message payload length;
                // if the length is exceeded, reject connection;
                // the bound can be application defined, i.e.
                // returned by `SharedData`
                read_buffer.clear();
                read_buffer.reserve(header.payload_length());
                read_buffer.resize(header.payload_length(), 0);

                // read the peer's payload
                if let Err(err) = socket.read_exact(&mut read_buffer[..header.payload_length()]) {
                    // errors reading -> faulty connection;
                    // drop this socket
                    error!("Failed to read payload from socket, faulty connection {:?}", err);
                    break;
                }

                // Use the threadpool for CPU intensive work in order to not block the IO threads
                let result = cpu_workers::deserialize_message::<RM, PM>(header.clone(),
                                                              read_buffer).recv().unwrap();

                let message = match result {
                    Ok((message, bytes)) => {
                        read_buffer = bytes;

                        message
                    }
                    Err(err) => {
                        // errors deserializing -> faulty connection;
                        // drop this socket
                        error!("Failed to deserialize message {:?}", err);
                        break;
                    }
                };

                match message {
                    NetworkMessageKind::ReconfigurationMessage(reconf_msg) => {
                        let msg = StoredMessage::new(header, reconf_msg.into());

                        if let Err(inner) = peer.reconf_handling.push_request(msg) {
                            error!("Failed to deliver reconfiguration message to reconfiguration handler. {:?}",
                            inner);
                        };
                    }
                    NetworkMessageKind::System(sys_msg) => {
                        let msg = StoredMessage::new(header, sys_msg.into());

                        if let Err(inner) = client_pool_rq.push_request(msg) {
                            error!("Channel closed, closing tcp connection as well to peer {:?}. {:?}",
                            peer_id,
                            inner);

                            break;
                        };
                    }
                    _ => {}
                }

                //TODO: Stats
            }

            let remaining_conns = peer.delete_connection(conn_handle.id());

            node_conns.handle_conn_lost(&peer.peer_node_id, remaining_conns)
        }).unwrap();
}