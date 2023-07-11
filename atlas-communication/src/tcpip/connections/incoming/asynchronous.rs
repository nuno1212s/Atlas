use std::sync::Arc;

use bytes::BytesMut;

use futures::AsyncReadExt;

use log::error;

use atlas_common::async_runtime as rt;
use atlas_common::socket::SecureReadHalfAsync;

use crate::cpu_workers;
use crate::message::{Header, NetworkMessageKind, StoredMessage};
use crate::reconfiguration_node::{NetworkInformationProvider, ReconfigurationMessageHandler};
use crate::serialize::Serializable;
use crate::tcpip::connections::{ConnHandle, PeerConnection, PeerConnections};

pub(super) fn spawn_incoming_task<NI, RM, PM>(
    conn_handle: ConnHandle,
    node_connections: Arc<PeerConnections<NI, RM, PM>>,
    peer: Arc<PeerConnection<RM, PM>>,
    mut socket: SecureReadHalfAsync)
    where
        NI: NetworkInformationProvider + 'static,
        RM: Serializable + 'static,
        PM: Serializable + 'static {
    rt::spawn(async move {
        let client_pool_buffer = Arc::clone(peer.client_pool_peer());
        let mut read_buffer = BytesMut::with_capacity(Header::LENGTH);
        let peer_id = client_pool_buffer.client_id().clone();

        loop {
            read_buffer.resize(Header::LENGTH, 0);

            if let Err(err) = socket.read_exact(&mut read_buffer[..Header::LENGTH]).await {
                // errors reading -> faulty connection;
                // drop this socket
                error!("{:?} // Failed to read header from socket, faulty connection {:?}", conn_handle.my_id, err);
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
            if let Err(err) = socket.read_exact(&mut read_buffer[..header.payload_length()]).await {
                // errors reading -> faulty connection;
                // drop this socket
                error!("{:?} // Failed to read payload from socket, faulty connection {:?}", conn_handle.my_id, err);
                break;
            }

            // Use the threadpool for CPU intensive work in order to not block the IO threads
            let result = cpu_workers::deserialize_message::<RM, PM>(header.clone(),
                                                          read_buffer).await.unwrap();

            let message = match result {
                Ok((message, bytes)) => {
                    read_buffer = bytes;

                    message
                }
                Err(err) => {
                    // errors deserializing -> faulty connection;
                    // drop this socket
                    error!("{:?} // Failed to deserialize message {:?}", conn_handle.my_id,err);
                    break;
                }
            };

            match message {
                NetworkMessageKind::System(sys_msg) => {
                    if let Err(inner) = client_pool_buffer.push_request(StoredMessage::new(header, sys_msg.into())) {
                        error!("{:?} // Channel closed, closing tcp connection as well to peer {:?}. {:?}",
                                    conn_handle.my_id,
                                    peer_id,
                                    inner,);

                        break;
                    };
                }
                NetworkMessageKind::ReconfigurationMessage(reconfig) => {
                    if let Err(err) = peer.reconf_handling.push_request(StoredMessage::new(header, reconfig.into())) {
                        error!("{:?} // Failed to push reconfiguration message to reconfiguration handler. {:?}", conn_handle.my_id, err);
                    }
                }
                _ => unreachable!()
            }

            //TODO: Statistics
        }

        let remaining_conns = peer.delete_connection(conn_handle.id());

        node_connections.handle_conn_lost(&peer.peer_node_id, remaining_conns);
    });
}
