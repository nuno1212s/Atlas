pub(crate) mod conn_establish;
pub mod epoll_group;
pub mod conn_util;

use crate::client_pooling::{ConnectedPeer, PeerIncomingRqHandling};
use crate::message::{StoredMessage, WireMessage};
use crate::mio_tcp::connections::conn_establish::{ConnectionHandler};
use crate::mio_tcp::connections::epoll_group::{
    EpollWorkerGroupHandle, EpollWorkerId, NewConnection,
};
use crate::reconfiguration_node::{NetworkInformationProvider, ReconfigurationMessageHandler};
use crate::serialize::Serializable;
use crate::NodeConnections;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx, OneShotRx, TryRecvError};
use atlas_common::error::*;
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::peer_addr::PeerAddr;
use atlas_common::socket::{MioSocket, SecureSocket, SecureSocketSync, SyncListener};
use crossbeam_skiplist::SkipMap;
use dashmap::DashMap;
use log::{debug, error, info, warn};
use mio::{Token, Waker};
use std::net::Shutdown;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use atlas_common::channel;
use crate::conn_utils::{Callback, ConnCounts};
use crate::mio_tcp::connections::conn_establish::pending_conn::{NetworkUpdateHandler, PendingConnHandle, RegisteredServers, ServerRegisteredPendingConns};
use crate::mio_tcp::connections::conn_util::{ReadingBuffer, WritingBuffer};

pub type NetworkSerializedMessage = (WireMessage);

pub const SEND_QUEUE_SIZE: usize = 1024;

pub struct Connections<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    id: NodeId,
    // The current pending connections, awaiting information from the reconfiguration protocol
    server_connections: Arc<ServerRegisteredPendingConns>,
    // The running servers, which are currently handling the reception of new connections from other nodes
    registered_servers: RegisteredServers,
    // The map of registered connections
    registered_connections: DashMap<NodeId, Arc<PeerConnection<RM, PM>>>,
    // A map of addresses to our known peers
    network_info: Arc<NI>,
    // A reference to the worker group that handles the epoll workers
    worker_group: EpollWorkerGroupHandle<RM, PM>,
    // A reference to the client pooling
    client_pooling: Arc<PeerIncomingRqHandling<StoredMessage<PM::Message>>>,
    // Reconfiguration message handling
    reconfig_handling: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    // Connection counts
    conn_counts: ConnCounts,
    // Handle establishing new connections
    conn_handler: Arc<ConnectionHandler>,
}

/// Structure that is responsible for handling all connections to a given peer
pub struct PeerConnection<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    node_type: NodeType,
    //A handle to the request buffer of the peer we are connected to in the client pooling module
    client: Arc<ConnectedPeer<StoredMessage<PM::Message>>>,
    //A handle to the reconfiguration message handler
    reconf_handling: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    // A thread-safe counter for generating connection ids
    conn_id_generator: AtomicU32,
    //The map connecting each connection to a token in the MIO Workers
    connections: SkipMap<u32, Option<ConnHandle>>,
    // Sending messages to the connections
    to_send: (
        ChannelSyncTx<NetworkSerializedMessage>,
        ChannelSyncRx<NetworkSerializedMessage>,
    ),
}

#[derive(Clone)]
pub struct ConnHandle {
    id: u32,
    my_id: NodeId,
    peer_id: NodeId,
    epoll_worker_id: EpollWorkerId,
    token: Token,
    waker: Arc<Waker>,
    pub(crate) cancelled: Arc<AtomicBool>,
}

impl<NI, RM, PM> NodeConnections for Connections<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static
{
    fn is_connected_to_node(&self, node: &NodeId) -> bool {
        self.registered_connections.contains_key(node)
    }

    fn connected_nodes_count(&self) -> usize {
        self.registered_connections.len()
    }

    fn connected_nodes(&self) -> Vec<NodeId> {
        self.registered_connections
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Attempt to connect to a given node
    fn connect_to_node(
        self: &Arc<Self>,
        node: NodeId,
    ) -> Vec<OneShotRx<Result<()>>> {
        if node == self.id {
            warn!("Attempted to connect to myself");

            return vec![];
        }

        let addr = self.get_addr_for_node(&node);
        let node_type = self.network_info.get_node_type(&node);

        if addr.is_none() || node_type.is_none() {
            error!("No address found for node {:?}", node);

            return vec![];
        }

        let addr = addr.unwrap();

        let current_connections = self
            .registered_connections
            .get(&node)
            .map(|entry| entry.value().concurrent_connection_count())
            .unwrap_or(0);

        let connections = self
            .conn_counts
            .get_connections_to_node(self.id, node, &*self.network_info);

        let connections = if current_connections > connections {
            0
        } else {
            connections - current_connections
        };

        let mut result_vec = Vec::with_capacity(connections);

        for _ in 0..connections {
            result_vec.push(
                self.conn_handler
                    .connect_to_node(Arc::clone(self), node, node_type.unwrap(), addr.clone()),
            )
        }

        result_vec
    }

    async fn disconnect_from_node(&self, node: &NodeId) -> Result<()> {
        let existing_connection = self.registered_connections.remove(node);

        if let Some((node, connection)) = existing_connection {
            for entry in connection.connections.iter() {
                if let Some(conn) = entry.value() {
                    let worker_id = conn.epoll_worker_id;
                    let conn_token = conn.token;

                    self.worker_group
                        .disconnect_connection_from_worker(worker_id, conn_token)?;
                }
            }
        }

        Ok(())
    }
}

impl<NI, RM, PM> Connections<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static
{
    pub(super) fn initialize_connections(
        id: NodeId,
        node_addr_lookup: Arc<NI>,
        group_handle: EpollWorkerGroupHandle<RM, PM>,
        conn_counts: ConnCounts,
        reconfiguration_handling: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
        client_pooling: Arc<PeerIncomingRqHandling<StoredMessage<PM::Message>>>,
    ) -> Result<Self> {
        let conn_handler = Arc::new(ConnectionHandler::initialize(
            id.clone(),
            conn_counts.clone(),
        ));

        let server_connections = Arc::new(ServerRegisteredPendingConns::new());

        let registered_servers = RegisteredServers::init();

        Ok(Self {
            id,
            server_connections,
            registered_servers,
            registered_connections: Default::default(),
            network_info: node_addr_lookup,
            worker_group: group_handle,
            client_pooling,
            reconfig_handling: reconfiguration_handling,
            conn_counts,
            conn_handler,
        })
    }



    pub(super) fn setup_tcp_server_worker(self: &Arc<Self>, listener: SyncListener) {
        let (tx, rx) = channel::new_bounded_sync(SEND_QUEUE_SIZE);

        self.registered_servers.register_server(tx);

        let waker = conn_establish::initialize_server(
            self.id.clone(),
            listener,
            self.conn_handler.clone(),
            self.server_connections.clone(),
            self.network_info.clone(),
            Arc::clone(self),
            self.reconfig_handling.clone(),
            rx
        );
    }

    /// Get the connection to a given node
    pub fn get_connection(&self, node: &NodeId) -> Option<Arc<PeerConnection<RM, PM>>> {
        let option = self.registered_connections.get(node);

        option.map(|conn| conn.value().clone())
    }

    /// Get the pending connection for a given node, if applicable
    pub fn get_pending_connection(&self, node: &NodeId) -> Option<PendingConnHandle> {
        self.server_connections.get_pending_conn(node)
    }

    /// Get the addr for the node given
    fn get_addr_for_node(&self, node: &NodeId) -> Option<PeerAddr> {
        self.network_info.get_addr_for_node(node)
    }

    /// Register a connection without having to provide any sockets, as this is meant to be done
    /// preemptively so there is no possibility for the connection details to be lost due to
    /// multi threading non atomic shenanigans
    fn preemptive_conn_register(self: &Arc<Self>, node: NodeId, node_type: NodeType, channel: (ChannelSyncTx<NetworkSerializedMessage>, ChannelSyncRx<NetworkSerializedMessage>))
                                -> Result<Arc<PeerConnection<RM, PM>>> {
        debug!("Preemptively registering connection to node {:?}", node);

        let option = self.registered_connections.entry(node);

        let conn = option.or_insert_with(|| {
            Arc::new(PeerConnection::new(node_type,
                                         self.client_pooling.init_peer_conn(node, node_type),
                                         self.reconfig_handling.clone(), channel))
        });

        Ok(conn.value().clone())
    }

    /// Handle a given socket having established the necessary connection
    fn handle_connection_established(self: &Arc<Self>, node: NodeId,
                                     socket: SecureSocket,
                                     node_type: NodeType,
                                     reading_info: ReadingBuffer,
                                     writing_info: Option<WritingBuffer>,
                                     channel: (ChannelSyncTx<NetworkSerializedMessage>, ChannelSyncRx<NetworkSerializedMessage>)) {
        let socket = match socket {
            SecureSocket::Sync(sync) => match sync {
                SecureSocketSync::Plain(socket) => socket,
                SecureSocketSync::Tls(tls, socket) => socket,
            },
            _ => unreachable!(),
        };

        self.handle_connection_established_with_socket(node, socket.into(), node_type, reading_info, writing_info, channel);
    }

    fn handle_connection_established_with_socket(
        self: &Arc<Self>,
        node: NodeId,
        socket: MioSocket,
        node_type: NodeType,
        reading_info: ReadingBuffer,
        writing_info: Option<WritingBuffer>,
        channel: (ChannelSyncTx<NetworkSerializedMessage>, ChannelSyncRx<NetworkSerializedMessage>),
    ) {
        info!(
            "{:?} // Handling established connection to {:?} with node type: {:?}",
            self.id, node, node_type
        );

        let option = self.registered_connections.entry(node);

        let peer_conn = option.or_insert_with(|| {
            let con = Arc::new(PeerConnection::new(
                node_type,
                self.client_pooling.init_peer_conn(node, node_type),
                self.reconfig_handling.clone(),
                channel,
            ));

            debug!(
                "{:?} // Creating new peer connection to {:?}. {:?}",
                self.id,
                node,
                con.client_pool_peer().client_id()
            );

            con
        });

        let concurrency_level =
            self.conn_counts
                .get_connections_to_node(self.id, node, &*self.network_info);

        let conn_id = peer_conn.gen_conn_id();

        let current_connections = peer_conn.concurrent_connection_count();

        //FIXME: Fix the fact that we are closing the previous connection when we don't actually need to
        // So now we have to multiply the limit because of this
        if current_connections + 1 > concurrency_level * 2 {
            // We have too many connections to this node. We need to close this one.
            warn!("{:?} // Too many connections to {:?}. Closing connection {:?}. Connection count {} vs max {}", self.id, node, conn_id,
            current_connections, concurrency_level);

            if let Err(err) = socket.shutdown(Shutdown::Both) {
                error!(
                    "{:?} // Failed to shutdown socket {:?} to {:?}. Error: {:?}",
                    self.id, conn_id, node, err
                );
            }

            return;
        }

        debug!(
            "{:?} // Registering connection {:?} to {:?}",
            self.id, conn_id, node
        );

        //FIXME: This isn't really an atomic operation but I also don't care LOL.
        peer_conn.register_peer_conn_intent(conn_id);

        let conn_details =
            NewConnection::new(conn_id, node, self.id, socket, reading_info, writing_info, peer_conn.value().clone());

        // We don't register the connection here as we still need some information that will only be provided
        // to us by the worker that will handle the connection.
        // Therefore, the connection will be registered in the worker itself.
        self.worker_group
            .assign_socket_to_worker(conn_details)
            .expect("Failed to assign socket to worker?");
    }

    /// Handle a connection having broken and being removed from the worker
    fn handle_connection_failed(self: &Arc<Self>, node: NodeId, conn_id: u32) {
        info!(
            "{:?} // Handling failed connection to {:?}. Conn: {:?}",
            self.id, node, conn_id
        );

        let connection = if let Some(conn) = self.registered_connections.get(&node) {
            conn.value().clone()
        } else {
            return;
        };

        connection.delete_connection(conn_id);

        if connection.concurrent_connection_count() == 0 {
            self.registered_connections.remove(&node);

            let _ = self.connect_to_node(node);
        }

    }
    pub fn pending_server_connections(&self) -> &Arc<ServerRegisteredPendingConns> {
        &self.server_connections
    }

    pub fn registered_servers(&self) -> &RegisteredServers {
        &self.registered_servers
    }
}

impl<RM, PM> PeerConnection<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static
{
    fn new(
        node_type: NodeType,
        client: Arc<ConnectedPeer<StoredMessage<PM::Message>>>,
        reconf_handling: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
        channel: (ChannelSyncTx<NetworkSerializedMessage>, ChannelSyncRx<NetworkSerializedMessage>),
    ) -> Self {
        Self {
            node_type,
            client,
            reconf_handling,
            conn_id_generator: AtomicU32::new(0),
            connections: Default::default(),
            to_send: channel,
        }
    }

    fn node_type(&self) -> NodeType {
        self.node_type
    }

    /// Get a unique ID for a connection
    fn gen_conn_id(&self) -> u32 {
        self.conn_id_generator.fetch_add(1, Ordering::Relaxed)
    }

    /// Register an active connection into this connection map
    fn register_peer_conn(&self, conn: ConnHandle) {
        self.connections.insert(conn.id, Some(conn));
    }

    // Register an intent of registering this connection
    fn register_peer_conn_intent(&self, id: u32) {
        self.connections.insert(id, None);
    }

    /// Get the amount of concurrent connections we currently have to this peer
    fn concurrent_connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Send the peer a given message
    pub(crate) fn peer_message(&self, msg: WireMessage, callback: Callback) -> Result<()> {
        let from = msg.header().from();
        let to = msg.header().to();

        if let Err(_) = self.to_send.0.send(msg) {
            error!("{:?} // Failed to send peer message to {:?}", from, to);

            return Err(Error::simple(ErrorKind::Communication));
        }

        for conn_ref in self.connections.iter() {
            let conn = conn_ref.value();

            if let Some(conn) = conn {
                conn.waker.wake().expect("Failed to wake connection");
            }
        }

        Ok(())
    }

    /// Take a message from the send queue (blocking)
    fn take_from_to_send(&self) -> Result<NetworkSerializedMessage> {
        self.to_send
            .1
            .recv()
            .wrapped(ErrorKind::CommunicationChannel)
    }

    /// Attempt to take a message from the send queue (non blocking)
    fn try_take_from_send(&self) -> Result<Option<NetworkSerializedMessage>> {
        match self.to_send.1.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(err) => match err {
                TryRecvError::ChannelDc => Err(Error::simple(ErrorKind::CommunicationChannel)),
                TryRecvError::ChannelEmpty | TryRecvError::Timeout => Ok(None),
            },
        }
    }

    fn delete_connection(&self, conn_id: u32) {
        self.connections.remove(&conn_id);
    }

    pub fn client_pool_peer(&self) -> &Arc<ConnectedPeer<StoredMessage<PM::Message>>> {
        &self.client
    }
}

impl ConnHandle {
    pub fn new(
        id: u32,
        my_id: NodeId,
        peer_id: NodeId,
        epoll_worker: EpollWorkerId,
        conn_token: Token,
        waker: Arc<Waker>,
    ) -> Self {
        Self {
            id,
            my_id,
            peer_id,
            epoll_worker_id: epoll_worker,
            cancelled: Arc::new(AtomicBool::new(false)),
            waker,
            token: conn_token,
        }
    }

    #[inline]
    pub fn id(&self) -> u32 {
        self.id
    }

    #[inline]
    pub fn my_id(&self) -> NodeId {
        self.my_id
    }

    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn peer_id(&self) -> NodeId {
        self.peer_id
    }

    #[inline]
    pub fn cancelled(&self) -> &Arc<AtomicBool> {
        &self.cancelled
    }
}
