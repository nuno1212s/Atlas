use std::io;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::sync::Arc;
use std::time::Duration;
use bytes::{Buf, Bytes, BytesMut};
use log::{error, info, trace};
use mio::{Events, Interest, Poll, Token, Waker};
use mio::event::Event;
use slab::Slab;
use atlas_common::channel::{ChannelSyncRx};
use atlas_common::error::{Error, ErrorKind, ResultWrappedExt};
use atlas_common::node_id::NodeId;
use atlas_common::socket::{MioSocket};
use crate::cpu_workers;
use crate::message::{Header, WireMessage};
use crate::mio_tcp::connections::{conn_util, Connections, ConnHandle};
use crate::mio_tcp::connections::conn_util::{ConnectionReadWork, ConnectionWriteWork, ReadingBuffer, WritingBuffer};
use crate::mio_tcp::connections::epoll_group::{EpollWorkerId, EpollWorkerMessage, NewConnection};
use crate::reconfiguration_node::NetworkInformationProvider;
use super::PeerConnection;
use crate::serialize::Serializable;

const EVENT_CAPACITY: usize = 1024;
const DEFAULT_SOCKET_CAPACITY: usize = 1024;
const WORKER_TIMEOUT: Option<Duration> = Some(Duration::from_millis(50));

enum ConnectionWorkResult {
    Working,
    ConnectionBroken,
}

type ConnectionRegister = ChannelSyncRx<MioSocket>;

/// The information for this worker thread.
pub(super) struct EpollWorker<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    // The id of this worker
    worker_id: EpollWorkerId,
    // A reference to our parent connections, so we can update it in case anything goes wrong
    // With any connections
    global_connections: Arc<Connections<NI, RM, PM>>,
    // This slab stores the connections that are currently being handled by this worker
    connections: Slab<SocketConnection<RM, PM>>,
    // register new connections
    connection_register: ChannelSyncRx<EpollWorkerMessage<RM, PM>>,
    // The poll instance of this worker
    poll: Poll,
    // Waker
    waker: Arc<Waker>,
    waker_token: Token,
}

/// All information related to a given connection
enum SocketConnection<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    PeerConn {
        // The handle of this connection
        handle: ConnHandle,
        // The mio socket that this connection refers to
        socket: MioSocket,
        // Information and buffers for the read end of this connection
        read_info: ReadingBuffer,
        // Information and buffers for the write end of this connection
        // Option since we may not be writing anything at the moment
        writing_info: Option<WritingBuffer>,
        // The connection to the peer this connection is a part of
        connection: Arc<PeerConnection<RM, PM>>,
    },
    Waker,
}

impl<NI, RM, PM> EpollWorker<NI, RM, PM>
    where
        NI: NetworkInformationProvider + 'static,
        RM: Serializable + 'static,
        PM: Serializable + 'static {
    /// Initializing a worker thread for the worker group
    pub fn new(worker_id: EpollWorkerId, connections: Arc<Connections<NI, RM, PM>>,
               register: ChannelSyncRx<EpollWorkerMessage<RM, PM>>) -> atlas_common::error::Result<Self> {
        let poll = Poll::new().wrapped_msg(ErrorKind::Communication, "Failed to initialize poll")?;

        let mut conn_slab = Slab::with_capacity(DEFAULT_SOCKET_CAPACITY);

        let entry = conn_slab.vacant_entry();

        let waker_token = Token(entry.key());
        let waker = Arc::new(Waker::new(poll.registry(), waker_token)
            .wrapped_msg(ErrorKind::Communication, "Failed to create waker")?);

        entry.insert(SocketConnection::Waker);

        info!("{:?} // Initialized Epoll Worker where Waker is token {:?}", connections.id, waker_token);

        Ok(Self {
            worker_id,
            global_connections: connections,
            connections: conn_slab,
            connection_register: register,
            poll,
            waker,
            waker_token,
        })
    }


    pub(super) fn epoll_worker_loop(mut self) -> io::Result<()> {
        let mut event_queue = Events::with_capacity(EVENT_CAPACITY);

        let my_id = self.global_connections.id;

        let waker_token = self.waker_token;

        loop {
            if let Err(e) = self.poll.poll(&mut event_queue, WORKER_TIMEOUT) {
                if e.kind() == io::ErrorKind::Interrupted {
                    // spurious wakeup
                    continue;
                } else if e.kind() == io::ErrorKind::TimedOut {
                    // *should* be handled by mio and return Ok() with no events
                    continue;
                } else {
                    return Err(e);
                }
            }

            trace!("{:?} // Worker {}: Handling {} events {:?}", self.global_connections.id, self.worker_id, event_queue.iter().count(), event_queue);

            for event in event_queue.iter() {
                if event.token() == waker_token {
                    // Indicates that we should try to write from the connections


                    // This is a bit of a hack, but we need to do this in order to avoid issues with the borrow
                    // Checker, since we would have to pass a mutable reference while holding immutable references.
                    // It's stupid but it is what it is
                    let mut to_verify = Vec::with_capacity(self.connections.len());

                    self.connections.iter().for_each(|(slot, conn)| {
                        let token = Token(slot);

                        if let SocketConnection::PeerConn { .. } = conn {
                            to_verify.push(token);
                        }
                    });

                    to_verify.into_iter().for_each(|token| {
                        match self.try_write_until_block(token) {
                            Ok(ConnectionWorkResult::ConnectionBroken) => {
                                let peer_id = {
                                    let connection = &self.connections[token.into()];

                                    connection.peer_id().unwrap_or(NodeId::from(1234567u32))
                                };

                                error!("{:?} // Connection broken during reading. Deleting connection {:?} to node {:?}",
                                    my_id, token,peer_id);

                                if let Err(err) = self.delete_connection(token, true) {
                                    error!("{:?} // Error deleting connection {:?} to node {:?}: {:?}",
                                        my_id, token, peer_id, err);
                                }
                            }
                            Ok(_) => {}
                            Err(err) => {
                                let peer_id = {
                                    let connection = &self.connections[token.into()];

                                    connection.peer_id().unwrap_or(NodeId::from(1234567u32))
                                };

                                error!("{:?} // Error handling connection event: {:?} for token {:?} (corresponding to conn id {:?})",
                                            my_id, err, token, peer_id);

                                if let Err(err) = self.delete_connection(token, true) {
                                    error!("{:?} // Error deleting connection {:?} to node {:?}: {:?}",
                                        my_id, token, peer_id, err);
                                }
                            }
                        };
                    });
                } else {
                    let token = event.token();

                    if !self.connections.contains(token.into()) {
                        // In case the waker already deleted this socket
                        continue;
                    }

                    match self.handle_connection_event(token, &event) {
                        Ok(ConnectionWorkResult::ConnectionBroken) => {
                            let peer_id = {
                                let connection = &self.connections[token.into()];

                                connection.peer_id().unwrap_or(NodeId::from(1234567u32))
                            };

                            error!("{:?} // Connection broken during reading. Deleting connection {:?} to node {:?}",
                                    self.global_connections.id, token,peer_id);

                            if let Err(err) = self.delete_connection(token, true) {
                                error!("{:?} // Error deleting connection {:?} to node {:?}: {:?}",
                                        my_id, token, peer_id, err);
                            }
                        }
                        Ok(_) => {}
                        Err(err) => {
                            let peer_id = {
                                let connection = &self.connections[token.into()];

                                connection.peer_id().unwrap_or(NodeId::from(1234567u32))
                            };

                            error!("{:?} // Error handling connection event: {:?} for token {:?} (corresponding to conn id {:?})",
                                            self.global_connections.id, err, token, peer_id);

                            if let Err(err) = self.delete_connection(token, true) {
                                error!("{:?} // Error deleting connection {:?} to node {:?}: {:?}",
                                        my_id, token, peer_id, err);
                            }
                        }
                    }
                }
            }

            self.register_connections()?;
        }
    }

    fn handle_connection_event(&mut self, token: Token, event: &Event) -> atlas_common::error::Result<ConnectionWorkResult> {
        let connection = if self.connections.contains(token.into()) {
            &self.connections[token.into()]
        } else {
            error!("{:?} // Received event for non-existent connection with token {:?}", self.global_connections.id, token);

            return Ok(ConnectionWorkResult::ConnectionBroken);
        };

        match &self.connections[token.into()] {
            SocketConnection::PeerConn { .. } => {
                if event.is_readable() {
                    if let ConnectionWorkResult::ConnectionBroken = self.read_until_block(token)? {
                        return Ok(ConnectionWorkResult::ConnectionBroken);
                    }
                }

                if event.is_writable() {
                    if let ConnectionWorkResult::ConnectionBroken = self.try_write_until_block(token)? {
                        return Ok(ConnectionWorkResult::ConnectionBroken);
                    }
                }
            }
            SocketConnection::Waker => {}
        }

        Ok(ConnectionWorkResult::Working)
    }

    fn try_write_until_block(&mut self, token: Token) -> atlas_common::error::Result<ConnectionWorkResult> {
        let connection = if self.connections.contains(token.into()) {
            &mut self.connections[token.into()]
        } else {
            error!("{:?} // Received write event for non-existent connection with token {:?}", self.global_connections.id, token);

            return Ok(ConnectionWorkResult::ConnectionBroken);
        };

        match connection {
            SocketConnection::PeerConn {
                socket,
                connection,
                writing_info,
                ..
            } => {
                let was_waiting_for_write = writing_info.is_some();
                let mut wrote = false;

                loop {
                    let writing = if let Some(writing_info) = writing_info {
                        wrote = true;

                        //We are writing something
                        writing_info
                    } else {
                        // We are not currently writing anything

                        if let Some(to_write) = connection.try_take_from_send()? {
                            trace!("{:?} // Writing message {:?}", self.global_connections.id, to_write);
                            wrote = true;

                            // We have something to write
                            *writing_info = Some(WritingBuffer::init_from_message(to_write)?);

                            writing_info.as_mut().unwrap()
                        } else {
                            // Nothing to write
                            trace!("{:?} // Nothing left to write, wrote? {}", self.global_connections.id, wrote);

                            // If we have written something in this loop but we have not written until
                            // Would block then we should flush the connection
                            if wrote {
                                match socket.flush() {
                                    Ok(_) => {}
                                    Err(ref err) if would_block(err) => break,
                                    Err(ref err) if interrupted(err) => continue,
                                    Err(err) => { return Err(Error::wrapped(ErrorKind::Communication, err)); }
                                };
                            }

                            break;
                        }
                    };

                    match conn_util::try_write_until_block(socket, writing)? {
                        ConnectionWriteWork::ConnectionBroken => {
                            return Ok(ConnectionWorkResult::ConnectionBroken);
                        }
                        ConnectionWriteWork::Working => { break; }
                        ConnectionWriteWork::Done => {
                            *writing_info = None;
                        }
                    }
                }

                if writing_info.is_none() && was_waiting_for_write {
                    // We have nothing more to write, so we no longer need to be notified of writability
                    self.poll.registry().reregister(socket, token, Interest::READABLE)
                        .wrapped_msg(ErrorKind::Communication, "Failed to reregister socket")?;
                } else if writing_info.is_some() && !was_waiting_for_write {
                    // We still have something to write but we reached a would block state,
                    // so we need to be notified of writability.
                    self.poll.registry().reregister(socket, token, Interest::READABLE.add(Interest::WRITABLE))
                        .wrapped_msg(ErrorKind::Communication, "Failed to reregister socket")?;
                } else {
                    // We have nothing to write and we were not waiting for writability, so we
                    // Don't need to re register
                    // Or we have something to write and we were already waiting for writability,
                    // So we also don't have to re register
                }
            }
            _ => unreachable!()
        }

        Ok(ConnectionWorkResult::Working)
    }

    fn read_until_block(&mut self, token: Token) -> atlas_common::error::Result<ConnectionWorkResult> {
        let connection = if self.connections.contains(token.into()) {
            &mut self.connections[token.into()]
        } else {
            error!("{:?} // Received read event for non-existent connection with token {:?}", self.global_connections.id, token);

            return Ok(ConnectionWorkResult::ConnectionBroken);
        };

        match connection {
            SocketConnection::PeerConn {
                handle,
                socket,
                read_info,
                connection,
                ..
            } => {
                match conn_util::read_until_block(socket, read_info)? {
                    ConnectionReadWork::ConnectionBroken => {
                        return Ok(ConnectionWorkResult::ConnectionBroken);
                    }
                    ConnectionReadWork::Working => { return Ok(ConnectionWorkResult::Working); }
                    ConnectionReadWork::WorkingAndReceived(received) | ConnectionReadWork::ReceivedAndDone(received) => {
                        for (header, message) in received {
                            cpu_workers::deserialize_and_push_message::<RM, PM>(header, message,
                                                                                connection.client.clone(),
                                                                                connection.reconf_handling.clone());
                        }
                    }
                }
                // We don't have any more
            }
            _ => unreachable!()
        }

        Ok(ConnectionWorkResult::Working)
    }

    /// Receive connections from the connection register and register them with the epoll instance
    fn register_connections(&mut self) -> io::Result<()> {
        loop {
            match self.connection_register.try_recv() {
                Ok(message) => {
                    match message {
                        EpollWorkerMessage::NewConnection(conn) => {
                            self.create_connection(conn)?;
                        }
                        EpollWorkerMessage::CloseConnection(token) => {
                            if let SocketConnection::Waker = &self.connections[token.into()] {
                                // We can't close the waker, wdym?
                                continue;
                            }

                            self.delete_connection(token, false)?;
                        }
                    }
                }
                Err(err) => {
                    // No more connections are ready to be accepted
                    break;
                }
            }
        }

        Ok(())
    }

    fn create_connection(&mut self, conn: NewConnection<RM, PM>) -> io::Result<()> {
        let NewConnection {
            conn_id, peer_id,
            my_id, mut socket,
            peer_conn
        } = conn;

        let entry = self.connections.vacant_entry();

        let token = Token(entry.key());

        let handle = ConnHandle::new(
            conn_id, my_id, peer_id,
            self.worker_id, token,
            self.waker.clone(),
        );

        peer_conn.register_peer_conn(handle.clone());

        self.poll.registry().register(&mut socket,
                                      token, Interest::READABLE)?;

        let socket_conn = SocketConnection::PeerConn {
            handle: handle.clone(),
            socket,
            read_info: ReadingBuffer::init_with_size(Header::LENGTH),
            writing_info: None,
            connection: peer_conn,
        };

        entry.insert(socket_conn);

        //TODO: Handle any errors from these calls
        let _ = self.read_until_block(token);
        let _ = self.try_write_until_block(token);

        Ok(())
    }

    fn delete_connection(&mut self, token: Token, is_failure: bool) -> io::Result<()> {
        if let Some(conn) = self.connections.try_remove(token.into()) {
            match conn {
                SocketConnection::PeerConn {
                    mut socket,
                    connection,
                    handle, ..
                } => {
                    self.poll.registry().deregister(&mut socket)?;

                    if is_failure {
                        self.global_connections.handle_connection_failed(handle.peer_id, handle.id);
                    } else {
                        connection.delete_connection(handle.id);
                    }

                    socket.shutdown(Shutdown::Both)?;
                }
                _ => unreachable!("Only peer connections can be removed from the connection slab")
            }
        } else {
            error!("{:?} // Tried to remove a connection that doesn't exist, {:?}", self.global_connections.id, token);
        }

        Ok(())
    }

    pub fn waker(&self) -> &Arc<Waker> {
        &self.waker
    }
}

pub fn would_block(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

pub fn interrupted(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Interrupted
}

impl<RM, PM> NewConnection<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    pub fn new(conn_id: u32, peer_id: NodeId, my_id: NodeId, socket: MioSocket, peer_conn: Arc<PeerConnection<RM, PM>>) -> Self {
        Self { conn_id, peer_id, my_id, socket, peer_conn }
    }
}

impl<RM, PM> SocketConnection<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    fn peer_id(&self) -> Option<NodeId> {
        match self {
            SocketConnection::PeerConn { handle, .. } => {
                Some(handle.peer_id)
            }
            SocketConnection::Waker => {
                None
            }
        }
    }
}