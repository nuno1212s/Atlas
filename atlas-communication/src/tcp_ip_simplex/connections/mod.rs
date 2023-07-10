pub mod conn_establish;
pub mod outgoing;
pub mod incoming;
mod ping_handler;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Instant;
use atlas_common::peer_addr::PeerAddr;
use dashmap::DashMap;
use log::{debug, error, warn};
use atlas_common::channel::{ChannelMixedRx, ChannelMixedTx, new_bounded_mixed, new_oneshot_channel, OneShotRx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::socket::SecureSocket;
use crate::client_pooling::{ConnectedPeer, PeerIncomingRqHandling};
use crate::message::{StoredMessage, WireMessage};
use crate::NodeConnections;
use crate::reconfiguration_node::{NetworkInformationProvider, ReconfigurationMessageHandler};
use crate::serialize::Serializable;
use crate::tcp_ip_simplex::connections::conn_establish::ConnectionHandler;
use crate::tcp_ip_simplex::connections::ping_handler::PingHandler;
use crate::tcpip::connections::{Callback, ConnCounts, ConnHandle, NetworkSerializedMessage};
use crate::tcpip::{NodeConnectionAcceptor, TlsNodeAcceptor, TlsNodeConnector};

/// How many slots the outgoing queue has for messages.
const TX_CONNECTION_QUEUE: usize = 1024;

pub struct SimplexConnections<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    id: NodeId,
    first_cli: NodeId,
    node_lookup: Arc<NI>,
    conn_counts: ConnCounts,
    client_pooling: Arc<PeerIncomingRqHandling<StoredMessage<PM::Message>>>,
    reconf_handling: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    connection_map: DashMap<NodeId, Arc<PeerConnection<RM, PM>>>,
    connection_establishing: Arc<ConnectionHandler>,
    ping_handler: Arc<PingHandler>,
}

pub struct PeerConnection<RM, PM>
    where
        RM: Serializable + 'static,
        PM: Serializable + 'static {
    peer_node_id: NodeId,
    //A handle to the request buffer of the peer we are connected to in the client pooling module
    client: Arc<ConnectedPeer<StoredMessage<PM::Message>>>,
    //The handler for reconfiguration messages
    reconf_handler: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    //The channel used to send serialized messages to the tasks that are meant to handle them
    tx: ChannelMixedTx<NetworkSerializedMessage>,
    // The RX handle corresponding to the tx channel above. This is so we can quickly associate new
    // TX connections to a given connection, as we just have to clone this handle
    rx: ChannelMixedRx<NetworkSerializedMessage>,
    // Counter to assign unique IDs to each of the underlying Tcp streams
    conn_id_generator: AtomicU32,
    // Controls the incoming connections
    outgoing_connections: Connections,
    // Controls the outgoing connections
    incoming_connections: Connections,
}

/// The connections of a given node (Only represent one way)
pub struct Connections {
    // A map to manage the currently active connections and a cached size value to prevent
    // concurrency for simple length checks
    active_connection_count: AtomicUsize,
    active_connections: Mutex<BTreeMap<u32, ConnHandle>>,
}

/// Which direction is a certain connection going in
enum ConnectionDirection {
    Incoming,
    Outgoing,
}

impl<NI, RM, PM> NodeConnections for SimplexConnections<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    fn is_connected_to_node(&self, node: &NodeId) -> bool {
        self.connection_map.contains_key(node)
    }

    fn connected_nodes_count(&self) -> usize {
        self.connection_map.len()
    }

    fn connected_nodes(&self) -> Vec<NodeId> {
        self.connection_map.iter().map(|val| {
            *val.key()
        }).collect()
    }

    fn connect_to_node(self: &Arc<Self>, node: NodeId) -> Vec<OneShotRx<Result<()>>> {
        let option = self.get_addr_for_node(node);

        match option {
            None => {
                let (tx, rx) = new_oneshot_channel();

                tx.send(Err(Error::simple(ErrorKind::CommunicationPeerNotFound))).unwrap();

                vec![rx]
            }
            Some(addr) => {
                let conns_to_have = self.conn_counts.get_connections_to_node(self.id, node, self.first_cli);
                let connections = self.current_connection_count_of(&node).unwrap_or(0);

                let mut oneshots = Vec::with_capacity(conns_to_have);

                for _ in connections..conns_to_have {
                    oneshots.push(self.connection_establishing.connect_to_node(self, node, addr.clone()));
                }

                oneshots
            }
        }
    }

    async fn disconnect_from_node(&self, node: &NodeId) -> Result<()> {
        if let Some((id, conn)) = self.connection_map.remove(node) {
            todo!();

            Ok(())
        } else {
            Err(Error::simple(ErrorKind::CommunicationPeerNotFound))
        }
    }
}

impl<NI, RM, PM> SimplexConnections<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    pub fn new(peer_id: NodeId, first_cli: NodeId,
               conn_counts: ConnCounts,
               addrs: Arc<NI>,
               node_connector: TlsNodeConnector,
               node_acceptor: TlsNodeAcceptor,
               client_pooling: Arc<PeerIncomingRqHandling<StoredMessage<PM::Message>>>,
               reconf_handling: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    ) -> Arc<Self> {
        let connection_establish = ConnectionHandler::new(peer_id, first_cli, conn_counts.clone(),
                                                          node_connector, node_acceptor);

        Arc::new(Self {
            id: peer_id,
            first_cli,
            node_lookup: addrs,
            connection_map: DashMap::new(),
            connection_establishing: connection_establish,
            client_pooling,
            conn_counts,
            ping_handler: PingHandler::new(),
            reconf_handling,
        })
    }

    fn get_addr_for_node(&self, node: NodeId) -> Option<PeerAddr> {
        self.node_lookup.get_addr_for_node(&node)
    }

    /// Setup a tcp listener inside this peer connections object.
    pub(super) fn setup_tcp_listener(self: Arc<Self>, node_acceptor: NodeConnectionAcceptor) {
        self.connection_establishing.clone().setup_conn_worker(node_acceptor, self)
    }

    /// Get the current amount of concurrent TCP connections between nodes
    pub fn current_connection_count_of(&self, node: &NodeId) -> Option<usize> {
        self.connection_map.get(node).map(|connection| {
            connection.value().connection_count()
        })
    }

    /// Get the connection to a given node
    pub fn get_connection(&self, node: &NodeId) -> Option<Arc<PeerConnection<RM, PM>>> {
        let option = self.connection_map.get(node);

        option.map(|conn| conn.value().clone())
    }

    /// Handle the connection being established
    fn handle_connection_established(self: &Arc<Self>, peer_id: NodeId, direction: ConnectionDirection, socket: SecureSocket) {
        debug!("{:?} // Handling established connection to {:?}", self.id, peer_id);

        let option = self.connection_map.entry(peer_id);

        let peer_conn = option.or_insert_with(||
            {
                let con = PeerConnection::new_peer(self.client_pooling.init_peer_conn(peer_id), self.reconf_handling.clone());

                debug!("{:?} // Creating new peer connection to {:?}. {:?}", self.id, peer_id,
                    con.client_pool_peer().client_id());

                con
            });

        let concurrency_level = self.conn_counts.get_connections_to_node(self.id, peer_id, self.first_cli);

        match direction {
            ConnectionDirection::Incoming => {
                // if we are the ones receiving the connection, then we have to attempt to also establish our TX side
                let mut current_outgoing_connections = peer_conn.outgoing_connection_count();

                debug!("Received incoming connection from {:?}. Establishing TX side with {} connections", peer_id, concurrency_level);

                if current_outgoing_connections > 0 {
                    let connections = peer_conn.outgoing_connections.active_connections.lock().unwrap();

                    for (id, _) in connections.iter() {
                        let _ = self.ping_handler.ping_peer(peer_id, id.clone());
                    }
                }

                while current_outgoing_connections < concurrency_level {
                    let addr = self.get_addr_for_node(peer_id).expect("Failed to get IP for node");

                    let _ = self.connection_establishing.connect_to_node(self, peer_id, addr.clone());

                    current_outgoing_connections += 1;
                }
            }
            ConnectionDirection::Outgoing => {
                // When we are establishing the connection, we don't have to re establish it xD
            }
        }

        peer_conn.insert_new_connection::<NI>(self.clone(), &self.ping_handler, socket, direction, concurrency_level);
    }

    /// Handle a connection that has been lost
    fn handle_conn_lost(self: &Arc<Self>, node: NodeId, remaining_conns: usize) {
        let concurrency_level = self.conn_counts.get_connections_to_node(self.id, node.clone(), self.first_cli);

        if remaining_conns <= 0 {
            //The node is no longer accessible. We will remove it until a new TCP connection
            // Has been established
            let _ = self.connection_map.remove(&node);
        }

        // Attempt to re-establish all of the missing connections
        if remaining_conns < concurrency_level {
            let addr = self.get_addr_for_node(node).expect("Failed to get IP for node");

            for _ in 0..concurrency_level - remaining_conns {
                self.connection_establishing.connect_to_node(self, node.clone(), addr.clone());
            }
        }
    }
}

impl<RM, PM> PeerConnection<RM, PM>
    where RM: Serializable + 'static, PM: Serializable + 'static {
    pub fn new_peer(client: Arc<ConnectedPeer<StoredMessage<PM::Message>>>,
                    reconf: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>) -> Arc<Self> {
        let (tx, rx) = new_bounded_mixed(TX_CONNECTION_QUEUE);

        Arc::new(Self {
            peer_node_id: client.client_id().clone(),
            client,
            reconf_handler: reconf,
            tx,
            rx,
            conn_id_generator: AtomicU32::new(0),
            outgoing_connections: Connections { active_connection_count: AtomicUsize::new(0), active_connections: Mutex::new(Default::default()) },
            incoming_connections: Connections { active_connection_count: AtomicUsize::new(0), active_connections: Mutex::new(Default::default()) },
        })
    }

    /// Get the amount of connections to this connected node
    fn connection_count(&self) -> usize {
        let active_incoming_conns = self.incoming_connections.active_connection_count.load(Ordering::Relaxed);

        let active_outgoing_conns = self.outgoing_connections.active_connection_count.load(Ordering::Relaxed);

        active_incoming_conns + active_outgoing_conns
    }

    /// Get the current amount of incoming connections
    fn incoming_connection_count(&self) -> usize {
        self.incoming_connections.active_connection_count.load(Ordering::Relaxed)
    }

    /// Get the current amount of outgoing connections
    fn outgoing_connection_count(&self) -> usize {
        self.outgoing_connections.active_connection_count.load(Ordering::Relaxed)
    }

    /// Insert a new connection
    fn insert_new_connection<NI>(self: &Arc<Self>,
                                 node_conns: Arc<SimplexConnections<NI, RM, PM>>,
                                 ping_handler: &Arc<PingHandler>,
                                 socket: SecureSocket,
                                 direction: ConnectionDirection, conn_limit: usize)
        where NI: NetworkInformationProvider + 'static {
        let conn_id = self.conn_id_generator.fetch_add(1, Ordering::Relaxed);

        let conn_handle = ConnHandle::new(conn_id, node_conns.id, self.peer_node_id);

        let mut conns = match direction {
            ConnectionDirection::Incoming => &self.incoming_connections,
            ConnectionDirection::Outgoing => &self.outgoing_connections
        };

        {
            let mut active_conns = conns.active_connections.lock().unwrap();

            active_conns.insert(conn_id, conn_handle.clone());
        }

        conns.active_connection_count.fetch_add(1, Ordering::Relaxed);

        match direction {
            ConnectionDirection::Incoming => {
                incoming::spawn_incoming_task_handler(conn_handle, node_conns, Arc::clone(self), socket)
            }
            ConnectionDirection::Outgoing => {
                outgoing::spawn_outgoing_task_handler(conn_handle, node_conns, Arc::clone(self), Arc::clone(ping_handler), socket);
            }
        }

        debug!("{:?} // Inserted new connection {:?} to {:?}. Current connection count: {:?}",
            self.peer_node_id, conn_id, self.peer_node_id, self.connection_count());
    }

    /// Delete a connection from this peers connection map
    fn delete_connection(&self, conn_id: u32, direction: ConnectionDirection) -> usize {
        // Remove the corresponding connection from the map
        let conn_handle = {
            let mut active_connections = match direction {
                ConnectionDirection::Incoming => self.incoming_connections.active_connections.lock(),
                ConnectionDirection::Outgoing => self.outgoing_connections.active_connections.lock()
            }.unwrap();

            // Do it inside a tiny scope to minimize the time the mutex is accessed
            active_connections.remove(&conn_id)
        };

        let active_connections = match direction {
            ConnectionDirection::Incoming => &self.incoming_connections.active_connection_count,
            ConnectionDirection::Outgoing => &self.outgoing_connections.active_connection_count
        };

        let remaining_conns = if let Some(conn_handle) = conn_handle {
            let conn_count = active_connections.fetch_sub(1, Ordering::Relaxed);

            //Setting the cancelled variable to true causes all associated threads to be
            //killed (as soon as they see the warning)
            conn_handle.cancelled.store(true, Ordering::Relaxed);

            conn_count
        } else {
            active_connections.load(Ordering::Relaxed)
        };

        warn!("Connection {} with peer {:?} has been deleted",
            conn_id,self.peer_node_id);

        remaining_conns
    }

    /// Send a message through this connection. Only valid for peer connections
    pub(crate) fn peer_message(&self, msg: WireMessage, callback: Callback, should_flush: bool, send_rq_time: Instant) -> Result<()> {
        let from = msg.header().from();
        let to = msg.header().to();

        if let Err(_) = self.tx.send((msg, callback, Instant::now(), should_flush, send_rq_time)) {
            error!("{:?} // Failed to send peer message to {:?}", from,
                to);

            return Err(Error::simple(ErrorKind::Communication));
        }

        Ok(())
    }

    async fn peer_msg_return_async(&self, to_send: NetworkSerializedMessage) -> Result<()> {
        let send = self.tx.clone();

        if let Err(_) = send.send_async(to_send).await {
            return Err(Error::simple(ErrorKind::Communication));
        }

        Ok(())
    }

    /// The client pool peer handle for the our peer connection
    pub fn client_pool_peer(&self) -> &Arc<ConnectedPeer<StoredMessage<PM::Message>>> {
        &self.client
    }

    /// Get the handle to the receiver for transmission
    fn to_send_handle(&self) -> &ChannelMixedRx<NetworkSerializedMessage> {
        &self.rx
    }
}