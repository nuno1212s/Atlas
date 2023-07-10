use std::sync::Arc;
use bytes::BytesMut;
use futures::AsyncReadExt;
use log::error;
use atlas_common::socket::SecureSocketAsync;
use atlas_common::error::*;
use atlas_common::async_runtime as rt;
use crate::cpu_workers;
use crate::cpu_workers::serialize_digest_threadpool_return_msg;
use crate::message::{Header, NetworkMessage, NetworkMessageKind, PingMessage, StoredMessage, WireMessage};
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::Serializable;
use crate::tcp_ip_simplex::connections::{ConnectionDirection, PeerConnection, SimplexConnections};
use crate::tcpip::connections::ConnHandle;

pub(super) fn spawn_incoming_task<NI, RM, PM>(
    conn_handle: ConnHandle,
    node_connections: Arc<SimplexConnections<NI, RM, PM>>,
    peer: Arc<PeerConnection<RM, PM>>,
    mut socket: SecureSocketAsync)
    where NI: NetworkInformationProvider + 'static,
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
                error!("{:?} // Failed to read header from socket, faulty connection {:?}", conn_handle.my_id(), err);
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
                error!("{:?} // Failed to read payload from socket, faulty connection {:?}", conn_handle.my_id(), err);
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
                    error!("{:?} // Failed to deserialize message {:?}", conn_handle.my_id(),err);
                    break;
                }
            };

            match message {
                NetworkMessageKind::Ping(ping) => {
                    if let Err(err) = respond_to_ping(&peer, &mut socket, &conn_handle, ping).await {
                        error!("{:?} // Failed to respond to ping {:?}", conn_handle.my_id(), err);
                        break;
                    }

                    continue;
                }
                NetworkMessageKind::ReconfigurationMessage(reconf) => {
                    let msg = StoredMessage::new(header, reconf.into());

                    if let Err(err) = peer.reconf_handler.push_request(msg) {
                        error!("{:?} // Failed to push reconfiguration message {:?}. {:?}",
                            conn_handle.my_id(),
                            peer_id,
                            err,);
                    }
                }
                NetworkMessageKind::System(sys_msg) => {
                    let msg = StoredMessage::new(header, sys_msg.into());

                    if let Err(inner) = client_pool_buffer.push_request(msg) {
                        error!("{:?} // Channel closed, closing tcp connection as well to peer {:?}. {:?}",
                            conn_handle.my_id(),
                            peer_id,
                            inner,);

                        break;
                    };
                }
            }

            //TODO: Statistics
        }

        let remaining_conns = peer.delete_connection(conn_handle.id(), ConnectionDirection::Incoming);

        // Retry to establish the connections if possible
        node_connections.handle_conn_lost(peer.peer_node_id, remaining_conns);
    });
}

async fn respond_to_ping<RM, PM>(peer: &Arc<PeerConnection<RM, PM>>,
                                 socket: &mut SecureSocketAsync,
                                 conn_handle: &ConnHandle, rq: PingMessage) -> Result<()>
    where RM: Serializable + 'static, PM: Serializable + 'static {
    if !rq.is_request() {
        return Err(Error::simple(ErrorKind::CommunicationPingHandler));
    }

    let pong = NetworkMessageKind::<RM, PM>::Ping(PingMessage::new(false));

    let (_, result) = serialize_digest_threadpool_return_msg(pong).await.wrapped(ErrorKind::CommunicationPingHandler)?;

    let (payload, digest) = result?;

    let nonce = fastrand::u64(..);

    let wm = WireMessage::new(conn_handle.my_id(), peer.peer_node_id, payload, nonce, Some(digest), None);

    wm.write_to(socket, true).await?;

    Ok(())
}