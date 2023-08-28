use std::collections::{BTreeMap};
use std::sync::{Arc, Mutex};
use atlas_common::peer_addr::PeerAddr;
use either::Either;
use atlas_common::channel::OneShotRx;
use atlas_common::socket::{AsyncSocket, SyncSocket};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::Serializable;
use crate::tcpip::{NodeConnectionAcceptor, TlsNodeAcceptor, TlsNodeConnector};
use crate::tcpip::connections::{ConnCounts, PeerConnections};

mod synchronous;
mod asynchronous;

/// Connection handler
pub struct ConnectionHandler {
    peer_id: NodeId,
    first_cli: NodeId,
    connector: TlsNodeConnector,
    tls_acceptor: TlsNodeAcceptor,
    concurrent_conn: ConnCounts,
    currently_connecting: Mutex<BTreeMap<NodeId, usize>>,
}

impl ConnectionHandler {
    pub fn new(peer_id: NodeId, first_cli: NodeId,
               conn_counts: ConnCounts,
               node_connector: TlsNodeConnector, node_acceptor: TlsNodeAcceptor) -> Arc<Self> {
        Arc::new(
            ConnectionHandler {
                peer_id,
                first_cli,
                connector: node_connector,
                tls_acceptor: node_acceptor,
                concurrent_conn: conn_counts,
                currently_connecting: Mutex::new(Default::default()),
            }
        )
    }

    pub(super) fn setup_conn_worker<NI, RM, PM>(self: Arc<Self>,
                                                listener: NodeConnectionAcceptor,
                                                peer_connections: Arc<PeerConnections<NI, RM, PM>>)
        where NI: NetworkInformationProvider + 'static,
              RM: Serializable + 'static,
              PM: Serializable + 'static {
        match listener {
            NodeConnectionAcceptor::Async(async_listener) => {
                asynchronous::setup_conn_acceptor_task(async_listener, self, peer_connections)
            }
            NodeConnectionAcceptor::Sync(sync_listener) => {
                synchronous::setup_conn_acceptor_thread(sync_listener, self, peer_connections)
            }
        }
    }

    pub fn id(&self) -> NodeId {
        self.peer_id
    }

    pub fn first_cli(&self) -> NodeId {
        self.first_cli
    }

    fn register_connecting_to_node(&self, peer_id: NodeId) -> bool {
        let mut connecting_guard = self.currently_connecting.lock().unwrap();

        let value = connecting_guard.entry(peer_id).or_insert(0);

        *value += 1;

        if *value > self.concurrent_conn.get_connections_to_node(self.id(), peer_id, self.first_cli) * 2 {
            *value -= 1;

            false
        } else {
            true
        }
    }

    fn done_connecting_to_node(&self, peer_id: &NodeId) {
        let mut connection_guard = self.currently_connecting.lock().unwrap();

        connection_guard.entry(peer_id.clone()).and_modify(|value| { *value -= 1 });

        if let Some(connection_count) = connection_guard.get(peer_id) {
            if *connection_count <= 0 {
                connection_guard.remove(peer_id);
            }
        }
    }

    pub fn connect_to_node<NI, RM, PM>(self: &Arc<Self>, peer_connections: &Arc<PeerConnections<NI, RM, PM>>,
                                       peer_id: NodeId, peer_addr: PeerAddr) -> OneShotRx<Result<()>>
        where NI: NetworkInformationProvider + 'static,
              RM: Serializable + 'static,
              PM: Serializable + 'static {
        match &self.connector {
            TlsNodeConnector::Async(_) => {
                asynchronous::connect_to_node_async(Arc::clone(self),
                                                    Arc::clone(&peer_connections),
                                                    peer_id, peer_addr)
            }
            TlsNodeConnector::Sync(_) => {
                synchronous::connect_to_node_sync(Arc::clone(self),
                                                  Arc::clone(&peer_connections),
                                                  peer_id, peer_addr)
            }
        }
    }

    pub fn accept_conn<NI, RM, PM>(self: &Arc<Self>, peer_connections: &Arc<PeerConnections<NI, RM, PM>>, socket: Either<AsyncSocket, SyncSocket>)
        where NI: NetworkInformationProvider + 'static,
              RM: Serializable + 'static,
              PM: Serializable + 'static {
        match socket {
            Either::Left(asynchronous) => {
                asynchronous::handle_server_conn_established(Arc::clone(self),
                                                             peer_connections.clone(),
                                                             asynchronous, );
            }
            Either::Right(synchronous) => {
                synchronous::handle_server_conn_established(Arc::clone(self),
                                                            peer_connections.clone(),
                                                            synchronous);
            }
        }
    }
}