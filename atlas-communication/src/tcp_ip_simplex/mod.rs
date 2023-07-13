pub mod connections;

use std::collections::BTreeMap;
use std::iter;
use std::sync::Arc;
use std::time::Instant;
use atlas_common::peer_addr::PeerAddr;
use either::Either;
use log::{debug, error};
use smallvec::SmallVec;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_common::prng::ThreadSafePrng;
use atlas_common::error::*;
use atlas_common::threadpool;
use atlas_common::crypto::signature::KeyPair;
use crate::client_pooling::{ConnectedPeer, PeerIncomingRqHandling};
use crate::config::{NodeConfig, TlsConfig};
use crate::message::{NetworkMessageKind, SerializedMessage, StoredMessage, StoredSerializedNetworkMessage, StoredSerializedProtocolMessage, WireMessage};
use crate::{FullNetworkNode};
use crate::protocol_node::ProtocolNetworkNode;
use crate::reconfiguration_node::{NetworkInformationProvider, ReconfigurationMessageHandler, ReconfigurationNode};
use crate::serialize::{Buf, Serializable};
use crate::tcp_ip_simplex::connections::{PeerConnection, SimplexConnections};
use crate::tcpip::connections::ConnCounts;
use crate::tcpip::{AsyncConn, ConnectionType, NodeConnectionAcceptor, TlsNodeAcceptor, TlsNodeConnector};

const NODE_QUORUM_SIZE: usize = 1024;

type SendTos<RM, PM> = SmallVec<[SendTo<RM, PM>; NODE_QUORUM_SIZE]>;

pub struct TCPSimplexNode<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    id: NodeId,
    first_cli: NodeId,
    // The thread safe pseudo random number generator
    rng: Arc<ThreadSafePrng>,
    /// General network information and reconfiguration logic
    reconfiguration: Arc<NI>,
    // The client pooling for this node
    client_pooling: Arc<PeerIncomingRqHandling<StoredMessage<PM::Message>>>,
    // The handle to the reconfiguration message handling
    reconfig_handle: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    // The manager of this peer's connections
    connections: Arc<SimplexConnections<NI, RM, PM>>,
}

impl<NI, RM, PM> TCPSimplexNode<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static
{
    async fn setup_client_facing_socket<T>(
        id: NodeId,
        addr: PeerAddr,
    ) -> Result<NodeConnectionAcceptor> where T: ConnectionType {
        debug!("{:?} // Attempt to setup client facing socket.", id);
        let server_addr = &addr.replica_facing_socket;

        T::setup_socket(&id, &server_addr.0).await
    }

    async fn setup_replica_facing_socket<T>(
        id: NodeId,
        peer_addr: PeerAddr,
    ) -> Result<Option<NodeConnectionAcceptor>>
        where T: ConnectionType {
        if let Some((socket, _)) = peer_addr.client_facing_socket {
            Ok(Some(T::setup_socket(&id, &socket).await?))
        } else {
            Ok(None)
        }
    }

    async fn setup_network<CT>(id: NodeId, addr: PeerAddr, cfg: TlsConfig) ->
    (TlsNodeConnector, TlsNodeAcceptor, Result<NodeConnectionAcceptor>, Result<Option<NodeConnectionAcceptor>>)
        where CT: ConnectionType
    {
        debug!("Initializing TLS configurations.");

        let async_acceptor: TlsAcceptor = Arc::new(cfg.async_server_config).into();
        let async_connector: TlsConnector = Arc::new(cfg.async_client_config).into();

        let sync_acceptor = Arc::new(cfg.sync_server_config);
        let sync_connector = Arc::new(cfg.sync_client_config);

        let connector = CT::setup_connector(sync_connector, async_connector);

        let acceptor = CT::setup_acceptor(sync_acceptor, async_acceptor);

        //Initialize the client facing server
        let client_listener = Self::setup_client_facing_socket::<CT>(id, addr.clone()).await;

        let replica_listener = Self::setup_replica_facing_socket::<CT>(id, addr.clone()).await;

        (connector, acceptor, client_listener, replica_listener)
    }


    /// Create the send tos for a given target
    fn send_tos(&self, shared: Option<&Arc<KeyPair>>, targets: impl Iterator<Item=NodeId>, flush: bool)
                -> (Option<SendTo<RM, PM>>, Option<SendTos<RM, PM>>, Vec<NodeId>) {
        let mut send_to_me = None;
        let mut send_tos: Option<SendTos<RM, PM>> = None;

        let mut failed = Vec::new();

        let my_id = self.id();

        let nonce = self.rng.next_state();

        for id in targets {
            if id == my_id {
                send_to_me = Some(SendTo {
                    my_id,
                    peer_id: id,
                    shared: shared.cloned(),
                    nonce,
                    reconf_msg: self.reconfig_handle.clone(),
                    peer_cnn: SendToPeer::Me(self.loopback_channel().clone()),
                    flush,
                    rq_send_time: Instant::now(),
                })
            } else {
                match self.connections.get_connection(&id) {
                    None => {
                        failed.push(id)
                    }
                    Some(conn) => {
                        if let Some(send_tos) = &mut send_tos {
                            send_tos.push(SendTo {
                                my_id,
                                peer_id: id.clone(),
                                shared: shared.cloned(),
                                nonce,
                                reconf_msg: self.reconfig_handle.clone(),
                                peer_cnn: SendToPeer::Peer(conn),
                                flush,
                                rq_send_time: Instant::now(),
                            })
                        } else {
                            let mut send = SmallVec::new();

                            send.push(SendTo {
                                my_id,
                                peer_id: id.clone(),
                                shared: shared.cloned(),
                                nonce,
                                reconf_msg: self.reconfig_handle.clone(),
                                peer_cnn: SendToPeer::Peer(conn),
                                flush,
                                rq_send_time: Instant::now(),
                            });

                            send_tos = Some(send)
                        }
                    }
                }
            }
        }

        (send_to_me, send_tos, failed)
    }

    fn serialize_send_impl(send_to_me: Option<SendTo<RM, PM>>, send_to_others: Option<SendTos<RM, PM>>,
                           message: NetworkMessageKind<RM, PM>) {
        threadpool::execute(move || {
            match crate::cpu_workers::serialize_digest_no_threadpool(&message) {
                Ok((buffer, digest)) => {
                    Self::send_impl(send_to_me, send_to_others, message, buffer, digest);
                }
                Err(err) => {
                    error!("Failed to serialize message {:?}", err);
                }
            }
        });
    }

    fn send_impl(send_to_me: Option<SendTo<RM, PM>>, send_to_others: Option<SendTos<RM, PM>>,
                 msg: NetworkMessageKind<RM, PM>, buffer: Buf, digest: Digest, ) {
        if let Some(send_to) = send_to_me {
            send_to.value(Either::Left((msg, buffer.clone(), digest.clone())));
        }

        if let Some(send_to) = send_to_others {
            for send in send_to {
                send.value(Either::Right((buffer.clone(), digest.clone())));
            }
        }
    }

    fn send_serialized_impl(send_to_me: Option<SendTo<RM, PM>>, send_to_others: Option<SendTos<RM, PM>>,
                            mut messages: BTreeMap<NodeId, StoredSerializedNetworkMessage<RM, PM>>) {
        if let Some(send_to) = send_to_me {
            let message = messages.remove(&send_to.peer_id).unwrap();

            send_to.value_serialized(message);
        }

        if let Some(send_to) = send_to_others {
            for send in send_to {
                let message = messages.remove(&send.peer_id).unwrap();

                send.value_serialized(message);
            }
        }
    }

    fn loopback_channel(&self) -> &Arc<ConnectedPeer<StoredMessage<PM::Message>>> {
        self.client_pooling.loopback_connection()
    }
}

impl<NI, RM, PM> ProtocolNetworkNode<PM> for TCPSimplexNode<NI, RM, PM>
    where NI: NetworkInformationProvider,
          RM: Serializable + 'static,
          PM: Serializable + 'static {

    type ConnectionManager = SimplexConnections<NI, RM, PM>;
    type NetworkInfoProvider = NI;
    type IncomingRqHandler = PeerIncomingRqHandling<StoredMessage<PM::Message>>;

    fn id(&self) -> NodeId {
        self.id
    }

    fn first_cli(&self) -> NodeId {
        self.first_cli
    }

    fn node_connections(&self) -> &Arc<Self::ConnectionManager> {
        &self.connections
    }

    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider> {
        &self.reconfiguration
    }

    fn node_incoming_rq_handling(&self) -> &Arc<Self::IncomingRqHandler> {
        &self.client_pooling
    }

    fn send(&self, message: PM::Message, target: NodeId, flush: bool) -> Result<()> {
        let message = NetworkMessageKind::from_system(message);

        let (send_to_me, send_to_others, failed) =
            self.send_tos(None, iter::once(target), flush);

        if !failed.is_empty() {
            return Err(Error::simple(ErrorKind::CommunicationPeerNotFound));
        }

        Self::serialize_send_impl(send_to_me, send_to_others, message);

        Ok(())
    }

    fn send_signed(&self, message: PM::Message, target: NodeId, flush: bool) -> Result<()> {
        let message = NetworkMessageKind::from_system(message);

        let shared = Some(self.reconfiguration.get_key_pair());

        let (send_to_me, send_to_others, failed) =
            self.send_tos(shared, iter::once(target), flush);

        if !failed.is_empty() {
            return Err(Error::simple(ErrorKind::CommunicationPeerNotFound));
        }

        Self::serialize_send_impl(send_to_me, send_to_others, message);

        Ok(())
    }

    fn broadcast(&self, message: PM::Message, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        let message = NetworkMessageKind::from_system(message);

        let (send_to_me, send_to_others, failed) =
            self.send_tos(None, targets, true);

        Self::serialize_send_impl(send_to_me, send_to_others, message);

        if !failed.is_empty() {
            Err(failed)
        } else {
            Ok(())
        }
    }

    fn broadcast_signed(&self, message: PM::Message, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        let shared = Some(self.reconfiguration.get_key_pair());

        let message = NetworkMessageKind::from_system(message);

        let (send_to_me, send_to_others, failed) =
            self.send_tos(shared, targets, true);

        Self::serialize_send_impl(send_to_me, send_to_others, message);

        if !failed.is_empty() {
            Err(failed)
        } else {
            Ok(())
        }
    }

    fn serialize_digest_message(&self, message: PM::Message) -> Result<(SerializedMessage<PM::Message>, Digest)> {
        let nmk = NetworkMessageKind::<RM, PM>::from_system(message);

        let key_pair = Some(&**self.reconfiguration.get_key_pair());

        let nonce = self.rng.next_state();

        match crate::cpu_workers::serialize_digest_no_threadpool(&nmk) {
            Ok((buffer, digest)) => {
                let msg = match nmk {
                    NetworkMessageKind::System(sys) => {
                        SerializedMessage::new(sys.into(), buffer)
                    }
                    _ => unreachable!()
                };

                Ok((msg, digest))
            }
            Err(err) => {
                error!("Failed to serialize message {:?}", err);

                Err(Error::simple(ErrorKind::CommunicationSerialize))
            }
        }
    }

    fn broadcast_serialized(&self, messages: BTreeMap<NodeId, StoredSerializedProtocolMessage<PM::Message>>) -> std::result::Result<(), Vec<NodeId>> {
        let targets = messages.keys().cloned().into_iter();

        let (send_to_me, send_to_others, failed) = self.send_tos(None,
                                                                 targets, true);

        let mut mapped_serialized_messages = BTreeMap::new();

        for (id, message) in messages.into_iter() {
            let (header, message) = message.into_inner();

            let (pm, buf) = message.into_inner();

            let nmk = NetworkMessageKind::from_system(pm);

            let message = StoredSerializedNetworkMessage::new(header, SerializedMessage::new(nmk, buf));

            mapped_serialized_messages.insert(id, message);
        }

        threadpool::execute(move || {
            Self::send_serialized_impl(send_to_me, send_to_others, mapped_serialized_messages);
        });

        if !failed.is_empty() {
            Err(failed)
        } else {
            Ok(())
        }
    }
}

impl<NI, RM, PM> ReconfigurationNode<RM> for TCPSimplexNode<NI, RM, PM>
    where NI: NetworkInformationProvider, RM: Serializable + 'static, PM: Serializable + 'static {
    type ConnectionManager = SimplexConnections<NI, RM, PM>;
    type NetworkInfoProvider = NI;
    type IncomingReconfigRqHandler = ReconfigurationMessageHandler<StoredMessage<RM::Message>>;

    fn node_connections(&self) -> &Arc<Self::ConnectionManager> {
        &self.connections
    }

    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider> {
        &self.reconfiguration
    }

    fn reconfiguration_message_handler(&self) -> &Arc<Self::IncomingReconfigRqHandler> {
        &self.reconfig_handle
    }

    fn send_reconfig_message(&self, message: RM::Message, target: NodeId) -> Result<()> {
        let nmk = NetworkMessageKind::from_reconfig(message);

        let keys = Some(self.reconfiguration.get_key_pair());

        let (send_to_me, send_to_others, failed) =
            self.send_tos(keys, iter::once(target), true);

        if !failed.is_empty() {
            return Err(Error::simple(ErrorKind::CommunicationPeerNotFound));
        }

        Self::serialize_send_impl(send_to_me, send_to_others, nmk);

        Ok(())
    }

    fn broadcast_reconfig_message(&self, message: RM::Message, target: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        let nmk = NetworkMessageKind::from_reconfig(message);

        let keys = Some(self.reconfiguration.get_key_pair());

        let (send_to_me, send_to_others, failed) =
            self.send_tos(keys, target, true);

        Self::serialize_send_impl(send_to_me, send_to_others, nmk);

        if !failed.is_empty() {
            Err(failed)
        } else {
            Ok(())
        }
    }
}

impl<NI, RM, PM> FullNetworkNode<NI, RM, PM> for TCPSimplexNode<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    type Config = NodeConfig;

    async fn bootstrap(network_info_provider: Arc<NI>, cfg: Self::Config) -> Result<Arc<Self>> {
        let id = cfg.id;

        debug!("Initializing sockets.");

        let tcp_config = cfg.tcp_config;

        let conn_counts = ConnCounts::from_tcp_config(&tcp_config);

        let reconfig_message_handler = Arc::new(ReconfigurationMessageHandler::initialize());

        let network = tcp_config.network_config;

        //Setup all the peer message reception handling.
        let peers = Arc::new(PeerIncomingRqHandling::new(
            cfg.id,
            cfg.first_cli,
            cfg.client_pool_config,
        ));

        let addr = network_info_provider.get_own_addr();

        let (connector, acceptor,
            client_socket, replica_socket) =
            Self::setup_network::<AsyncConn>(id, addr.clone(), network).await;

        let peer_connections = SimplexConnections::new(id, cfg.first_cli,
                                                       conn_counts,
                                                       network_info_provider.clone(),
                                                       connector,
                                                       acceptor,
                                                       peers.clone(),
                                                       reconfig_message_handler.clone());


        debug!("Initializing connection listeners");
        peer_connections.clone().setup_tcp_listener(client_socket?);

        if let Some(replica) = replica_socket? {
            peer_connections.clone().setup_tcp_listener(replica);
        }

        let rng = Arc::new(ThreadSafePrng::new());

        debug!("{:?} // Initializing node reference", id);

        let node = Arc::new(TCPSimplexNode {
            id,
            first_cli: cfg.first_cli,
            rng,
            reconfiguration: network_info_provider,
            client_pooling: peers,
            reconfig_handle: reconfig_message_handler,
            connections: peer_connections,
        });

        // success
        Ok(node)
    }
}

/// Some information about a message about to be sent to a peer
struct SendTo<RM, PM> where RM: Serializable + 'static, PM: Serializable + 'static {
    my_id: NodeId,
    peer_id: NodeId,
    shared: Option<Arc<KeyPair>>,
    nonce: u64,
    reconf_msg: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    peer_cnn: SendToPeer<RM, PM>,
    flush: bool,
    rq_send_time: Instant,
}

/// The information about the connection itself which can either be a loopback
/// or a peer connection
enum SendToPeer<RM, PM> where RM: Serializable + 'static, PM: Serializable + 'static {
    Me(Arc<ConnectedPeer<StoredMessage<PM::Message>>>),
    Peer(Arc<PeerConnection<RM, PM>>),
}

impl<RM, PM> SendTo<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    fn value(self, msg: Either<(NetworkMessageKind<RM, PM>, Buf, Digest), (Buf, Digest)>) {
        let key_pair = match &self.shared {
            None => {
                None
            }
            Some(key_pair) => {
                Some(&**key_pair)
            }
        };

        match (self.peer_cnn, msg) {
            (SendToPeer::Me(conn), Either::Left((msg, buf, digest))) => {
                let message = WireMessage::new(self.my_id, self.peer_id,
                                               buf, self.nonce, Some(digest), key_pair);

                let (header, _) = message.into_inner();

                match msg {
                    NetworkMessageKind::ReconfigurationMessage(reconfig_msg) => {
                        self.reconf_msg.push_request(StoredMessage::new(header, reconfig_msg.into())).unwrap();
                    }
                    NetworkMessageKind::System(sys_msg) => {
                        conn.push_request(StoredMessage::new(header, sys_msg.into())).unwrap();
                    }
                    _ => {
                        unreachable!()
                    }
                }
            }
            (SendToPeer::Peer(peer), Either::Right((buf, digest))) => {
                let message = WireMessage::new(self.my_id, self.peer_id,
                                               buf, self.nonce, Some(digest), key_pair);

                peer.peer_message(message, None, true, Instant::now()).unwrap();
            }
            (_, _) => { unreachable!() }
        }
    }

    fn value_serialized(self, msg: StoredSerializedNetworkMessage<RM, PM>) {
        match self.peer_cnn {
            SendToPeer::Me(peer_conn) => {
                let (header, msg) = msg.into_inner();

                let (msg, _) = msg.into_inner();

                match msg {
                    NetworkMessageKind::ReconfigurationMessage(reconfig_msg) => {
                        self.reconf_msg.push_request(StoredMessage::new(header, reconfig_msg.into())).unwrap();
                    }
                    NetworkMessageKind::System(sys_msg) => {
                        peer_conn.push_request(StoredMessage::new(header, sys_msg.into())).unwrap();
                    }
                    _ => {
                        unreachable!()
                    }
                }
            }
            SendToPeer::Peer(peer_cnn) => {
                let (header, msg) = msg.into_inner();

                let (_, buf) = msg.into_inner();

                let wm = WireMessage::from_parts(header, buf).unwrap();

                peer_cnn.peer_message(wm, None, true, Instant::now()).unwrap();
            }
        }
    }
}