#![allow(dead_code, clippy::large_enum_variant)]

use crate::conn_util;
use crate::conn_util::{
    interrupted, would_block, ConnectionReadWork, ConnectionWriteWork, ReadMessageError,
    ReadingBuffer, WritingBuffer, WritingBufferError,
};
use crate::connections::{ByteMessageSendStub, ConnHandle, Connections, MioError, PeerConn};
use crate::epoll::{EpollWorkerId, EpollWorkerMessage, NewConnection};
use atlas_common::channel::sync::ChannelSyncRx;
use atlas_common::node_id::NodeId;
use atlas_common::socket::MioSocket;
use atlas_common::Err;
use atlas_communication::byte_stub::{NodeIncomingStub, NodeStubController};
use std::error::Error;
use std::fmt::{Debug, Formatter};

use atlas_communication::reconfiguration::NetworkInformationProvider;
use mio::event::Event;
use mio::{Events, Interest, Poll, Token, Waker};
use slab::Slab;
use std::io;
use std::io::Write;
use std::net::Shutdown;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info};

const EVENT_CAPACITY: usize = 1024;
const DEFAULT_SOCKET_CAPACITY: usize = 1024;
const WORKER_TIMEOUT: Option<Duration> = Some(Duration::from_millis(50));

#[derive(Debug)]
enum ConnectionWorkResult {
    Working,
    ConnectionBroken(usize, usize),
}

type ConnectionRegister = ChannelSyncRx<MioSocket>;

pub(crate) struct EpollWorker<NI, CN, CNP>
where
    NI: NetworkInformationProvider,
{
    worker_id: EpollWorkerId,

    global_conns: Arc<Connections<NI, CN, CNP>>,

    connections: Slab<SocketConnection<CN>>,

    conn_register: ChannelSyncRx<EpollWorkerMessage<CN>>,

    // MIO related stuff
    poll: Poll,
    waker: Arc<Waker>,
    waker_token: Token,
}

enum SocketConnection<CN> {
    PeerConn {
        handle: ConnHandle,
        socket: MioSocket,
        reading_info: ReadingBuffer,
        writing_info: Option<WritingBuffer>,
        connection: Arc<PeerConn<CN>>,
    },
    Waker,
}

impl<CN> SocketConnection<CN> {
    pub(crate) fn peer_id(&self) -> Option<NodeId> {
        match self {
            SocketConnection::PeerConn { handle, .. } => Some(handle.peer_id()),
            SocketConnection::Waker => None,
        }
    }
}

impl<NI, CN, CNP> EpollWorker<NI, CN, CNP>
where
    CN: NodeIncomingStub + 'static,
    NI: NetworkInformationProvider + 'static,
    CNP: NodeStubController<ByteMessageSendStub, CN> + 'static,
{
    /// Initializing a worker thread for the worker group
    pub(crate) fn new(
        worker_id: EpollWorkerId,
        connections: Arc<Connections<NI, CN, CNP>>,
        register: ChannelSyncRx<EpollWorkerMessage<CN>>,
    ) -> Result<Self, io::Error> {
        let poll = Poll::new()?;

        let mut conn_slab = Slab::with_capacity(DEFAULT_SOCKET_CAPACITY);

        let entry = conn_slab.vacant_entry();

        let waker_token = Token(entry.key());
        let waker = Arc::new(Waker::new(poll.registry(), waker_token)?);

        entry.insert(SocketConnection::Waker);

        info!(
            "{:?} // Initialized Epoll Worker where Waker is token {:?}",
            connections.own_id(),
            waker_token
        );

        Ok(Self {
            worker_id,
            global_conns: connections,
            connections: conn_slab,
            conn_register: register,
            poll,
            waker,
            waker_token,
        })
    }

    //#[instrument(level = Level::DEBUG)]
    pub(super) fn epoll_worker_loop(mut self) -> Result<(), WorkerLoopError<CN::Error>> {
        let mut event_queue = Events::with_capacity(EVENT_CAPACITY);

        let my_id = self.global_conns.own_id();

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
                    return Err(e.into());
                }
            }

            for event in event_queue.iter() {
                if event.token() == waker_token {
                    // Indicates that we should try to write from the connections

                    // This is a bit of a hack, but we need to do this in order to avoid issues with the borrow
                    // Checker, since we would have to pass a mutable reference while holding immutable references.
                    // It's stupid but it is what it is

                    let to_verify = self
                        .connections
                        .iter()
                        .filter_map(|(slot, conn)| {
                            let token = Token(slot);

                            if let SocketConnection::PeerConn { .. } = conn {
                                Some(token)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();

                    for token in to_verify {
                        match self.try_write_until_block(token) {
                            Ok(ConnectionWorkResult::ConnectionBroken(written, to_write)) => {
                                let peer_id = {
                                    let connection = &self.connections[token.into()];

                                    connection.peer_id().unwrap_or(NodeId::from(1234567u32))
                                };

                                error!(" Connection broken during writing after waker token. Deleting connection {:?} to node {:?}
                            Connection broken at written {:?} bytes, and had {:?} bytes left to write",
                                    token,peer_id, written, to_write);
                                if let Err(err) = self.delete_connection(token, true) {
                                    error!(
                                        "{:?} // Error deleting connection {:?} to node {:?}: {:?}",
                                        my_id, token, peer_id, err
                                    );
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
                                    error!(
                                        "{:?} // Error deleting connection {:?} to node {:?}: {:?}",
                                        my_id, token, peer_id, err
                                    );
                                }
                            }
                        };
                    }
                } else {
                    let token = event.token();

                    if !self.connections.contains(token.into()) {
                        // In case the waker already deleted this socket
                        continue;
                    }

                    match self.handle_connection_event(token, event) {
                        Ok(ConnectionWorkResult::ConnectionBroken(written, to_write)) => {
                            let peer_id = {
                                let connection = &self.connections[token.into()];

                                connection.peer_id().unwrap_or(NodeId::from(1234567u32))
                            };

                            error!("Connection broken during handling of connection event {:?}. Deleting connection {:?} to node {:?}.\
                            Connection broken at written {:?} bytes, and had {:?} bytes left to write",
                                    event, token,peer_id, written, to_write);

                            if let Err(err) = self.delete_connection(token, true) {
                                error!(
                                    "{:?} // Error deleting connection {:?} to node {:?}: {:?}",
                                    my_id, token, peer_id, err
                                );
                            }
                        }
                        Ok(_) => {}
                        Err(err) => {
                            let peer_id = {
                                let connection = &self.connections[token.into()];

                                connection.peer_id().unwrap_or(NodeId::from(1234567u32))
                            };

                            error!("{:?} // Error handling connection event: {:?} for token {:?} (corresponding to conn id {:?})",
                                            self.global_conns.own_id(), err, token, peer_id);

                            if let Err(err) = self.delete_connection(token, true) {
                                error!(
                                    "{:?} // Error deleting connection {:?} to node {:?}: {:?}",
                                    my_id, token, peer_id, err
                                );
                            }
                        }
                    }
                }
            }

            self.register_connections()?;
        }
    }

    //#[instrument(level = Level::DEBUG)]
    fn handle_connection_event(
        &mut self,
        token: Token,
        event: &Event,
    ) -> Result<ConnectionWorkResult, HandleConnEventError<CN::Error>> {
        let _connection = if self.connections.contains(token.into()) {
            &self.connections[token.into()]
        } else {
            error!(
                "{:?} // Received event for non-existent connection with token {:?}",
                self.global_conns.own_id(),
                token
            );

            return Err(HandleConnEventError::ReceivedEventForNonExistentConnection(
                token,
            ));
        };

        match &self.connections[token.into()] {
            SocketConnection::PeerConn { .. } => {
                if event.is_readable() {
                    if let ConnectionWorkResult::ConnectionBroken(written, to_write) =
                        self.read_until_block(token)?
                    {
                        return Ok(ConnectionWorkResult::ConnectionBroken(written, to_write));
                    }
                }

                if event.is_writable() {
                    if let ConnectionWorkResult::ConnectionBroken(written, to_write) =
                        self.try_write_until_block(token)?
                    {
                        return Ok(ConnectionWorkResult::ConnectionBroken(written, to_write));
                    }
                }
            }
            SocketConnection::Waker => {}
        }

        Ok(ConnectionWorkResult::Working)
    }

    //#[instrument(level = Level::TRACE)]
    fn try_write_until_block(&mut self, token: Token) -> Result<ConnectionWorkResult, WriteError> {
        let connection = if self.connections.contains(token.into()) {
            &mut self.connections[token.into()]
        } else {
            error!(
                "{:?} // Received write event for non-existent connection with token {:?}",
                self.global_conns.own_id(),
                token
            );

            return Err(WriteError::ReceivedReadEventForNonExistentConnection(token));
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
                            wrote = true;

                            // We have something to write
                            *writing_info = Some(WritingBuffer::init_from_message(to_write)?);

                            writing_info.as_mut().unwrap()
                        } else {
                            // If we have written something in this loop but we have not written until
                            // Would block then we should flush the connection
                            if wrote {
                                match socket.flush() {
                                    Ok(_) => {}
                                    Err(ref err) if would_block(err) => break,
                                    Err(ref err) if interrupted(err) => continue,
                                    Err(err) => {
                                        return Err!(err);
                                    }
                                };
                            }

                            break;
                        }
                    };

                    match conn_util::try_write_until_block(socket, writing)? {
                        ConnectionWriteWork::ConnectionBroken(written, to_write) => {
                            return Ok(ConnectionWorkResult::ConnectionBroken(written, to_write));
                        }
                        ConnectionWriteWork::Working => {
                            break;
                        }
                        ConnectionWriteWork::Done => {
                            *writing_info = None;
                        }
                    }
                }

                if writing_info.is_none() && was_waiting_for_write {
                    // We have nothing more to write, so we no longer need to be notified of writability
                    self.poll
                        .registry()
                        .reregister(socket, token, Interest::READABLE)?;
                } else if writing_info.is_some() && !was_waiting_for_write {
                    // We still have something to write but we reached a would block state,
                    // so we need to be notified of writability.
                    self.poll.registry().reregister(
                        socket,
                        token,
                        Interest::READABLE.add(Interest::WRITABLE),
                    )?;
                } else {
                    // We have nothing to write and we were not waiting for writability, so we
                    // Don't need to re register
                    // Or we have something to write and we were already waiting for writability,
                    // So we also don't have to re register
                }
            }
            _ => unreachable!(),
        }

        Ok(ConnectionWorkResult::Working)
    }

    //#[instrument(level = Level::TRACE)]
    fn read_until_block(
        &mut self,
        token: Token,
    ) -> Result<ConnectionWorkResult, ReadError<CN::Error>> {
        let connection = if self.connections.contains(token.into()) {
            &mut self.connections[token.into()]
        } else {
            error!(
                "{:?} // Received read event for non-existent connection with token {:?}",
                self.global_conns.own_id(),
                token
            );

            return Err(ReadError::ReceivedReadEventForNonExistentConnection);
        };

        match connection {
            SocketConnection::PeerConn {
                socket,
                reading_info,
                connection,
                ..
            } => {
                match conn_util::read_until_block(socket, reading_info)? {
                    ConnectionReadWork::ConnectionBroken(read, to_read) => {
                        return Ok(ConnectionWorkResult::ConnectionBroken(read, to_read));
                    }
                    ConnectionReadWork::Working => {
                        return Ok(ConnectionWorkResult::Working);
                    }
                    ConnectionReadWork::WorkingAndReceived(received)
                    | ConnectionReadWork::ReceivedAndDone(received) => {
                        // Handle the messages that we have received
                        // In this case by propagating them upwards in the architecture
                        for message in received {
                            connection
                                .byte_input_stub()
                                .handle_message(self.global_conns.network_info(), message)
                                .map_err(ReadError::HandleMessageError)?;
                        }
                    }
                }
                // We don't have any more
            }
            _ => unreachable!(),
        }

        Ok(ConnectionWorkResult::Working)
    }

    /// Receive connections from the connection register and register them with the epoll instance
    //#[instrument(level = Level::TRACE)]
    fn register_connections(&mut self) -> Result<(), RegisterConnectionError<CN::Error>> {
        while let Ok(message) = self.conn_register.try_recv() {
            match message {
                EpollWorkerMessage::NewConnection(conn) => {
                    self.create_connection(*conn)?;
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

        Ok(())
    }

    //#[instrument(level = Level::TRACE)]
    fn create_connection(
        &mut self,
        conn: NewConnection<CN>,
    ) -> Result<(), CreateConnectionError<CN::Error>> {
        let NewConnection {
            conn_id,
            peer_id,
            my_id,
            mut socket,
            reading_info,
            writing_info,
            peer_conn,
        } = conn;

        let entry = self.connections.vacant_entry();

        let token = Token(entry.key());

        let handle = ConnHandle::new(
            conn_id,
            my_id,
            peer_id,
            self.worker_id,
            token,
            self.waker.clone(),
        );

        peer_conn.register_peer_conn(handle.clone());

        self.poll
            .registry()
            .register(&mut socket, token, Interest::READABLE)?;

        let socket_conn = SocketConnection::PeerConn {
            handle: handle.clone(),
            socket,
            reading_info,
            writing_info,
            connection: peer_conn,
        };

        entry.insert(socket_conn);

        debug!(
            "{:?} // Registered new connection {:?} to node {:?} in epoll worker {:?}",
            self.global_conns.own_id(),
            token,
            peer_id,
            self.worker_id
        );

        self.read_until_block(token)?;
        self.try_write_until_block(token)?;

        Ok(())
    }

    //#[instrument(level = Level::DEBUG)]
    fn delete_connection(
        &mut self,
        token: Token,
        is_failure: bool,
    ) -> Result<(), DeleteConnectionError> {
        if let Some(conn) = self.connections.try_remove(token.into()) {
            match conn {
                SocketConnection::PeerConn {
                    mut socket,
                    connection,
                    handle,
                    ..
                } => {
                    self.poll.registry().deregister(&mut socket)?;

                    if is_failure {
                        self.global_conns
                            .handle_connection_failed(handle.peer_id(), handle.id());
                    } else {
                        connection.delete_connection(handle.id());
                    }

                    info!(
                        "{:?} // Deleted connection {:?} to node {:?}",
                        self.global_conns.own_id(),
                        token,
                        handle.peer_id()
                    );

                    socket.shutdown(Shutdown::Both)?;
                }
                _ => unreachable!("Only peer connections can be removed from the connection slab"),
            }
        } else {
            error!(
                "{:?} // Tried to remove a connection that doesn't exist, {:?}",
                self.global_conns.own_id(),
                token
            );
        }

        Ok(())
    }

    pub fn waker(&self) -> &Arc<Waker> {
        &self.waker
    }
}

impl<CN> Debug for NewConnection<CN> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewConnection")
            .field("conn_id", &self.conn_id)
            .field("peer_id", &self.peer_id)
            .field("my_id", &self.my_id)
            .finish_non_exhaustive()
    }
}

impl<NI, CN, CNP> Debug for EpollWorker<NI, CN, CNP>
where
    NI: NetworkInformationProvider,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpollWorker")
            .field("worker_id", &self.worker_id)
            .field("managed_conns", &self.connections.len())
            .finish_non_exhaustive()
    }
}

#[derive(Error, Debug)]
pub(crate) enum WorkerLoopError<NIE>
where
    NIE: Error,
{
    #[error("Failed to read event loop due to IO Error: {0}")]
    IoError(#[from] io::Error),
    #[error("Failed to register pending connections due to error: {0}")]
    RegisteredConnectionsError(#[from] RegisterConnectionError<NIE>),
}

#[derive(Error, Debug)]
pub(crate) enum RegisterConnectionError<NIE>
where
    NIE: Error,
{
    #[error("Failed to create connection due to: {0}")]
    CreateConnectionError(#[from] CreateConnectionError<NIE>),
    #[error("Failed to delete connection due to: {0}")]
    FailedToDeleteConnectionError(#[from] DeleteConnectionError),
}

#[derive(Error, Debug)]
pub(crate) enum CreateConnectionError<CNE>
where
    CNE: Error,
{
    #[error("Failed to create connection due to error while reading: {0}")]
    Read(#[from] ReadError<CNE>),
    #[error("Failed to create connection due to error while writing: {0}")]
    Write(#[from] WriteError),
    #[error("Failed to register connection due to IO Error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Error, Debug)]
pub(crate) enum HandleConnEventError<CNE>
where
    CNE: Error,
{
    #[error("Received event for non-existent connection")]
    ReceivedEventForNonExistentConnection(Token),
    #[error("Failed to create connection due to error while reading: {0}")]
    ReadError(#[from] ReadError<CNE>),
    #[error("Failed to create connection due to error while writing: {0}")]
    WriteError(#[from] WriteError),
}

#[derive(Error, Debug)]
pub(crate) enum ReadError<CNE>
where
    CNE: Error,
{
    #[error("Received read event for non-existent connection")]
    ReceivedReadEventForNonExistentConnection,
    #[error("Failed to handle message due to internal error {0}")]
    HandleMessageError(CNE),
    #[error("Failed to read message from network layer due to {0}")]
    ReadMessageError(#[from] ReadMessageError),
}

#[derive(Error, Debug)]
pub(crate) enum WriteError {
    #[error("Received write event for non-existent connection {0}")]
    FailedToReRegisterSocket(#[from] io::Error),
    #[error("Received write event for non-existent connection for token {0:?}")]
    ReceivedReadEventForNonExistentConnection(Token),
    #[error("Failed to get message {0}")]
    FailedToTake(#[from] MioError),
    #[error("Failed to initialize writing buffer")]
    FailedToInitWritingBuffer(#[from] WritingBufferError),
}

#[derive(Error, Debug)]
#[error("Failed to shutdown socket {0:?}")]
pub(crate) struct DeleteConnectionError(#[from] io::Error);
