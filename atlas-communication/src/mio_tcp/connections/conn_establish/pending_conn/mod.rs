use std::sync::{Arc, Mutex};

use dashmap::DashMap;
use log::info;
use mio::Waker;

use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;

use crate::message::{StoredMessage, WireMessage};
use crate::mio_tcp::connections::{Connections, NetworkSerializedMessage};
use crate::reconfiguration_node::{NetworkInformationProvider, NetworkUpdateMessage, ReconfigurationMessageHandler};
use crate::serialize::Serializable;

/// A handle to a pending node's connection
#[derive(Clone)]
pub struct PendingConnHandle {
    id: NodeId,
    channel: (ChannelSyncTx<NetworkSerializedMessage>, ChannelSyncRx<NetworkSerializedMessage>),
    /// A correct node is not going to connect to the 2 different servers at the same time,
    /// even if he does, we can just use one server's connection to send the reconfiguration messages,
    /// So this should be just fine
    waker: Arc<Waker>,
}

/// The pending connections that we still have not received from the reconfiguration protocol
pub struct ServerRegisteredPendingConns {
    pending_conns: DashMap<NodeId, PendingConnHandle>,
}

/// A message to be delivered to the server threads for them to transition the pending connections
/// to active connections
pub struct NetworkUpdate {
    conn_handle: PendingConnHandle,
    network_update: NetworkUpdateMessage,
}

/// handle updates sent from the reconfiguration protocol and then propagate them to the threads
/// responsible for managing the currently pending connections, so they
pub(crate) struct NetworkUpdateHandler<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    server_conns: Arc<ServerRegisteredPendingConns>,
    registered_servers: RegisteredServers,
    reconfiguration_handler: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    peer_conns: Arc<Connections<NI, RM, PM>>,
}

/// The servers that are registered with us and that should receive updates from the reconfiguration protocol
#[derive(Clone)]
pub struct RegisteredServers {
    registered_servers: Arc<Mutex<Vec<ChannelSyncTx<NetworkUpdate>>>>,
}

impl<RM, PM, NI> NetworkUpdateHandler<NI, RM, PM>
    where
        NI: NetworkInformationProvider + 'static,
        RM: Serializable + 'static,
        PM: Serializable + 'static {
    pub fn initialize_update_handler(
        registered_servers: RegisteredServers,
        pending_conns: Arc<ServerRegisteredPendingConns>,
        reconf_handle: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
        conns: Arc<Connections<NI, RM, PM>>) {
        let handler = Self {
            server_conns: pending_conns,
            registered_servers,
            reconfiguration_handler: reconf_handle,
            peer_conns: conns,
        };

        std::thread::Builder::new()
            .name(format!("Network Update Handler Thread"))
            .spawn(move || {
                handler.run();
            }).expect("Failed to spawn NetworkUpdateHandler thread");
    }

    pub fn registered_servers(&self) -> RegisteredServers {
        self.registered_servers.clone()
    }

    fn run(self) {
        loop {
            let network_update = self.reconfiguration_handler.receive_network_update();

            if let Ok(network_update) = network_update {
                match &network_update {
                    NetworkUpdateMessage::NodeConnectionPermitted(node_id, node_type, pk) => {
                        match self.server_conns.get_pending_conn(&node_id) {
                            None => {
                                unreachable!("Received a connection permitted message for a node that is not pending connection. Node: {:?}", node_id)
                            }
                            Some(conn) => {
                                info!("Received a connection permitted message for node {:?} of type {:?}", node_id, node_type);

                                // Register the new connection
                                self.peer_conns.preemptive_conn_register(node_id.clone(),
                                                                         node_type.clone(),
                                                                         conn.channel.clone()).expect("Failed to preemptively register the connection?");

                                // By only removing the pending connection after the preemptive registration,
                                // we ensure that the connection is not removed from the pending connections before the preemptive registration is complete
                                self.server_conns.remove_pending_connection(node_id);

                                self.registered_servers.registered_servers.lock().unwrap().iter().for_each(|tx|
                                    tx.send(NetworkUpdate {
                                        conn_handle: conn.clone(),
                                        network_update: network_update.clone(),
                                    }).unwrap()
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

impl NetworkUpdate {
    pub fn into_inner(self) -> (PendingConnHandle, NetworkUpdateMessage) {
        (self.conn_handle, self.network_update)
    }
}

impl PendingConnHandle {
    pub(crate) fn new(id: NodeId, channel: (ChannelSyncTx<NetworkSerializedMessage>, ChannelSyncRx<NetworkSerializedMessage>),
                      waker: Arc<Waker>) -> Self {
        Self { id, channel, waker }
    }

    pub fn peer_message(&self, message: WireMessage) -> Result<()> {
        match self.channel.0.send(message) {
            Ok(_) => {}
            Err(err) => {
                return Err(Error::simple_with_msg(ErrorKind::CommunicationChannel, format!("Failed to send message to channel. Error: {:?}", err).as_str()));
            }
        }

        self.waker.wake().wrapped_msg(ErrorKind::CommunicationServerNotWoken, "Failed to wake the server thread")
    }

    pub fn channel(&self) -> &(ChannelSyncTx<NetworkSerializedMessage>, ChannelSyncRx<NetworkSerializedMessage>) {
        &self.channel
    }
}

impl ServerRegisteredPendingConns {
    pub fn new() -> Self {
        Self { pending_conns: DashMap::new() }
    }

    pub fn get_pending_conn(&self, node: &NodeId) -> Option<PendingConnHandle> {
        self.pending_conns.get(node).map(|conn| conn.value().clone())
    }

    pub fn insert_pending_connection(&self, conn: PendingConnHandle) {
        self.pending_conns.insert(conn.id, conn);
    }

    pub fn remove_pending_connection(&self, node_id: &NodeId) -> PendingConnHandle {
        self.pending_conns.remove(node_id).unwrap().1
    }
}

impl RegisteredServers {
    pub fn init() -> Self {
        Self {
            registered_servers: Arc::new(Mutex::new(vec![])),
        }
    }

    pub fn register_server(&self, tx: ChannelSyncTx<NetworkUpdate>) {
        self.registered_servers.lock().unwrap().push(tx);
    }
}
