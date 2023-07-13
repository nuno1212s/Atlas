use std::collections::BTreeMap;
use std::iter;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use either::Either;
use log::{debug, error};
use smallvec::SmallVec;

use atlas_common::{socket, threadpool};
use atlas_common::crypto::hash::Digest;
use atlas_common::crypto::signature::KeyPair;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::peer_addr::PeerAddr;
use atlas_common::prng::ThreadSafePrng;
use atlas_common::socket::SyncListener;
use atlas_metrics::metrics::metric_duration;

use crate::{FullNetworkNode};
use crate::client_pooling::{ConnectedPeer, PeerIncomingRqHandling};
use crate::config::MioConfig;
use crate::message::{NetworkMessageKind, SerializedMessage, StoredMessage, StoredSerializedNetworkMessage, StoredSerializedProtocolMessage, WireMessage};
use crate::metric::THREADPOOL_PASS_TIME_ID;
use crate::mio_tcp::connections::{Connections, PeerConnection};
use crate::mio_tcp::connections::epoll_group::{init_worker_group_handle, initialize_worker_group};
use crate::protocol_node::ProtocolNetworkNode;
use crate::reconfiguration_node::{NetworkInformationProvider, ReconfigurationMessageHandler, ReconfigurationNode};
use crate::serialize::{Buf, Serializable};
use crate::tcpip::connections::ConnCounts;

mod connections;

const NODE_QUORUM_SIZE: usize = 32;

type SendTos<RM, PM> = SmallVec<[SendTo<RM, PM>; NODE_QUORUM_SIZE]>;

/// The node that handles the TCP connections
pub struct MIOTcpNode<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    id: NodeId,
    first_cli: NodeId,
    // The thread safe random number generator
    rng: Arc<ThreadSafePrng>,
    /// General network information and reconfiguration logic
    reconfiguration: Arc<NI>,
    // The connections that are currently being maintained by us to other peers
    connections: Arc<Connections<NI, RM, PM>>,
    // Handles the incoming reconfiguration messages, which will be handled separately from the
    // Rest of the protocol requests
    reconfig_handling: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
    //Handles the incoming connections' buffering and request collection
    //This is polled by the proposer for client requests and by the
    client_pooling: Arc<PeerIncomingRqHandling<StoredMessage<PM::Message>>>,
}

impl<NI, RM, PM> MIOTcpNode<NI, RM, PM>
    where
        NI: NetworkInformationProvider + 'static,
        RM: Serializable + 'static,
        PM: Serializable + 'static {
    fn setup_connection(id: &NodeId, server_addr: &SocketAddr) -> Result<SyncListener> {
        socket::bind_sync_server(server_addr.clone()).wrapped(ErrorKind::Communication)
    }

    fn setup_client_facing_socket(
        id: NodeId,
        addr: PeerAddr,
    ) -> Result<SyncListener> {
        debug!("{:?} // Attempt to setup client facing socket.", id);
        let server_addr = &addr.replica_facing_socket;

        Self::setup_connection(&id, &server_addr.0)
    }

    fn setup_replica_facing_socket(
        id: NodeId,
        peer_addr: PeerAddr,
    ) -> Result<Option<SyncListener>> {
        if let Some((socket, _)) = peer_addr.client_facing_socket {
            Ok(Some(Self::setup_connection(&id, &socket)?))
        } else {
            Ok(None)
        }
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
                    reconfig_handling: self.reconfig_handling.clone(),
                    peer_cnn: SendToPeer::Me(self.client_pooling.loopback_connection().clone()),
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
                                reconfig_handling: self.reconfig_handling.clone(),
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
                                reconfig_handling: self.reconfig_handling.clone(),
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
        let start = Instant::now();

        threadpool::execute(move || {
            metric_duration(THREADPOOL_PASS_TIME_ID, start.elapsed());

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
}

impl<NI, RM, PM> ProtocolNetworkNode<PM> for MIOTcpNode<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    type ConnectionManager = Connections<NI, RM, PM>;
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
        let nmk = NetworkMessageKind::from_system(message);

        let (send_to_me, send_to_others, failed) =
            self.send_tos(None, iter::once(target), flush);

        if !failed.is_empty() {
            return Err(Error::simple(ErrorKind::CommunicationPeerNotFound));
        }

        Self::serialize_send_impl(send_to_me, send_to_others, nmk);

        Ok(())
    }

    fn send_signed(&self, message: PM::Message, target: NodeId, flush: bool) -> Result<()> {
        let nmk = NetworkMessageKind::from_system(message);

        let keys = Some(self.reconfiguration.get_key_pair());

        let (send_to_me, send_to_others, failed) =
            self.send_tos(keys, iter::once(target), flush);

        if !failed.is_empty() {
            return Err(Error::simple(ErrorKind::CommunicationPeerNotFound));
        }

        Self::serialize_send_impl(send_to_me, send_to_others, nmk);

        Ok(())
    }

    fn broadcast(&self, message: PM::Message, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        let nmk = NetworkMessageKind::from_system(message);

        let (send_to_me, send_to_others, failed) =
            self.send_tos(None, targets, true);

        Self::serialize_send_impl(send_to_me, send_to_others, nmk);

        if !failed.is_empty() {
            Err(failed)
        } else {
            Ok(())
        }
    }

    fn broadcast_signed(&self, message: PM::Message, target: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        let nmk = NetworkMessageKind::from_system(message);

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

impl<NI, RM, PM> ReconfigurationNode<RM> for MIOTcpNode<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    type ConnectionManager = Connections<NI, RM, PM>;
    type NetworkInfoProvider = NI;
    type IncomingReconfigRqHandler = ReconfigurationMessageHandler<StoredMessage<RM::Message>>;

    fn node_connections(&self) -> &Arc<Self::ConnectionManager> {
        &self.connections
    }

    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider> {
        &self.reconfiguration
    }

    fn reconfiguration_message_handler(&self) -> &Arc<Self::IncomingReconfigRqHandler> {
        &self.reconfig_handling
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

impl<NI, RM, PM> FullNetworkNode<NI, RM, PM> for MIOTcpNode<NI, RM, PM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          PM: Serializable + 'static {
    type Config = MioConfig;

    async fn bootstrap(network_info_provider: Arc<NI>, node_config: Self::Config) -> Result<Arc<Self>> where NI: NetworkInformationProvider {
        let MioConfig { node_config: cfg, worker_count } = node_config;

        let id = cfg.id;

        debug!("Initializing sockets.");

        let tcp_config = cfg.tcp_config;

        let conn_counts = ConnCounts::from_tcp_config(&tcp_config);

        let reconfig_message_handler = Arc::new(ReconfigurationMessageHandler::initialize());

        let network = tcp_config.network_config;

        let rng = Arc::new(ThreadSafePrng::new());

        debug!("{:?} // Initializing node reference", id);

        //Setup all the peer message reception handling.
        let peers = Arc::new(PeerIncomingRqHandling::new(
            cfg.id,
            cfg.first_cli,
            cfg.client_pool_config,
        ));

        let (handle, receivers) = init_worker_group_handle::<NI, RM, PM>(worker_count as u32);

        let connections = Arc::new(Connections::initialize_connections(
            cfg.id,
            cfg.first_cli,
            network_info_provider.clone(),
            handle.clone(),
            conn_counts.clone(),
            reconfig_message_handler.clone(),
            peers.clone(),
        )?);

        initialize_worker_group(connections.clone(), receivers)?;

        let addr = network_info_provider.get_own_addr();

        let client_listener = Self::setup_client_facing_socket(cfg.id.clone(), addr.clone())?;

        let replica_listener = Self::setup_replica_facing_socket(cfg.id.clone(), addr.clone())?;

        connections.setup_tcp_server_worker(client_listener);

        replica_listener.map(|listener| connections.setup_tcp_server_worker(listener));

        let network_node = Arc::new(Self {
            id,
            first_cli: cfg.first_cli,
            rng,
            connections,
            reconfig_handling: reconfig_message_handler,
            client_pooling: peers,
            reconfiguration: network_info_provider.clone(),
        });

        Ok(network_node)
    }
}

/// Some information about a message about to be sent to a peer
struct SendTo<RM, PM>
    where RM: Serializable + 'static,
          PM: Serializable + 'static {
    my_id: NodeId,
    peer_id: NodeId,
    shared: Option<Arc<KeyPair>>,
    nonce: u64,
    reconfig_handling: Arc<ReconfigurationMessageHandler<StoredMessage<RM::Message>>>,
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
                        self.reconfig_handling.push_request(StoredMessage::new(header, reconfig_msg.into())).unwrap();
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

                peer.peer_message(message, None).unwrap();
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
                        self.reconfig_handling.push_request(StoredMessage::new(header, reconfig_msg.into())).unwrap();
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

                peer_cnn.peer_message(wm, None).unwrap();
            }
        }
    }
}