use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use either::Either;
use getset::{CopyGetters, Getters};
use smallvec::SmallVec;
use tracing::{error, warn};

use atlas_common::crypto::hash::Digest;
use atlas_common::crypto::signature::KeyPair;
use atlas_common::node_id::NodeId;
use atlas_common::prng::ThreadSafePrng;
use atlas_common::quiet_unwrap;
use atlas_metrics::metrics::{metric_duration, metric_store_count_max};

use crate::byte_stub::outgoing::PeerOutgoingConnection;
use crate::byte_stub::peer_conn_manager::PeerConnectionManager;
use crate::byte_stub::{ByteNetworkDispatchError, ByteNetworkStub};
use crate::lookup_table::{LookupTable, MessageModule, ModMessageWrapped};
use crate::message::{Buf, Header, StoredSerializedMessage, WireMessage};
use crate::metric::{
    COMM_SERIALIZE_SIGN_TIME_ID, MESSAGE_DELIVER_TIME_ID, OUTGOING_MESSAGE_SIZE_ID,
    THREADPOOL_PASS_TIME_ID,
};
use crate::reconfiguration::NetworkInformationProvider;
use crate::serialization;
use crate::serialization::Serializable;

const NODE_QUORUM_SIZE: usize = 32;

type SendTos<CN, R, O, S, A> = SmallVec<[SendTo<CN, R, O, S, A>; NODE_QUORUM_SIZE]>;

#[derive(Getters, CopyGetters)]
pub struct SendTo<CN, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    from: NodeId,
    #[get = "pub"]
    to: NodeId,
    shared: Option<Arc<KeyPair>>,
    nonce: u64,
    peer: PeerOutgoingConnection<CN, R, O, S, A>,
    #[get_copy = "pub(crate)"]
    authenticated_state: bool,
}

impl<CN, R, O, S, A> SendTo<CN, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    CN: Clone,
{
    pub fn initialize_send_tos_serialized<NI, L>(
        conn_manager: &PeerConnectionManager<NI, CN, R, O, S, A, L>,
        shared: Option<&Arc<KeyPair>>,
        rng: &Arc<ThreadSafePrng>,
        target: NodeId,
    ) -> (Option<Self>, Option<Self>)
    where
        L: Clone + Send,
        NI: NetworkInformationProvider,
    {
        let mut send_to_me = None;
        let mut send_tos = None;

        let mut failed_lookup = Vec::new();

        let nonce = rng.next_state();

        let option = conn_manager.get_connection_to_node(&target);

        match option {
            None => {
                failed_lookup.push(target);
            }
            Some(stub) => {
                let connection = stub.outgoing_connection().clone();

                let send_to = SendTo {
                    from: conn_manager.node_id(),
                    to: target,
                    shared: shared.cloned(),
                    nonce,
                    peer: connection,
                    authenticated_state: stub.is_authenticated(),
                };

                if target == conn_manager.node_id() {
                    send_to_me = Some(send_to);
                } else {
                    send_tos = Some(send_to);
                }
            }
        }

        (send_to_me, send_tos)
    }

    pub type InitializedSendTos = (Option<Self>, Option<SendTos<CN, R, O, S, A>>);

    pub fn initialize_send_tos<NI, L>(
        conn_manager: &PeerConnectionManager<NI, CN, R, O, S, A, L>,
        shared: Option<&Arc<KeyPair>>,
        rng: &Arc<ThreadSafePrng>,
        targets: impl Iterator<Item = NodeId>,
    ) -> Self::InitializedSendTos
    where
        L: LookupTable<R, O, S, A>,
        NI: NetworkInformationProvider,
    {
        let mut send_to_me = None;
        let mut send_tos = Some(SmallVec::new());

        let mut failed_lookup = Vec::new();

        let nonce = rng.next_state();

        targets.for_each(|target| {
            let option = conn_manager.get_connection_to_node(&target);

            match option {
                None => {
                    failed_lookup.push(target);
                }
                Some(stub) => {
                    let connection = stub.outgoing_connection().clone();

                    let send_to = SendTo {
                        from: conn_manager.node_id(),
                        to: target,
                        shared: shared.cloned(),
                        nonce,
                        peer: connection,
                        authenticated_state: stub.is_authenticated(),
                    };

                    if target == conn_manager.node_id() {
                        send_to_me = Some(send_to);
                    } else {
                        if send_tos.is_none() {
                            send_tos = Some(SmallVec::new());
                        }

                        send_tos.as_mut().unwrap().push(send_to);
                    }
                }
            }
        });

        (send_to_me, send_tos)
    }

    pub type TypedMessage = (ModMessageWrapped<R, O, S, A>, Buf, Digest);
    pub type SerializedMessage = (MessageModule, Buf, Digest);

    pub fn value(
        self,
        msg: Either<Self::TypedMessage, Self::SerializedMessage>,
        start_time: Instant,
    ) where
        CN: ByteNetworkStub + 'static,
    {
        let key_pair: Option<&KeyPair> = None;

        match (self.peer, msg) {
            (PeerOutgoingConnection::LoopbackStub(stub), Either::Left((msg, buf, digest))) => {
                let wire_msg = WireMessage::new(
                    self.from,
                    self.to,
                    msg.get_module(),
                    buf,
                    self.nonce,
                    Some(digest),
                    key_pair,
                );

                metric_duration(COMM_SERIALIZE_SIGN_TIME_ID, start_time.elapsed());

                let deliver_message_start_time = Instant::now();

                let (header, _, _) = wire_msg.into_inner();

                stub.handle_message(header, msg);
                metric_duration(
                    MESSAGE_DELIVER_TIME_ID,
                    deliver_message_start_time.elapsed(),
                );
            }
            (PeerOutgoingConnection::OutgoingStub(stub), Either::Right((msg_mod, buf, digest))) => {
                let wire_msg = WireMessage::new(
                    self.from,
                    self.to,
                    msg_mod,
                    buf,
                    self.nonce,
                    Some(digest),
                    key_pair,
                );

                metric_duration(COMM_SERIALIZE_SIGN_TIME_ID, start_time.elapsed());

                let deliver_message_start_time = Instant::now();

                dispatch_message(stub, wire_msg, self.to);

                metric_duration(
                    MESSAGE_DELIVER_TIME_ID,
                    deliver_message_start_time.elapsed(),
                );
            }
            _ => unreachable!(),
        }
    }

    pub fn value_ser(self, msg: StoredSerializedMessage<ModMessageWrapped<R, O, S, A>>)
    where
        CN: ByteNetworkStub + 'static,
    {
        match (self.peer, msg) {
            (
                PeerOutgoingConnection::LoopbackStub(stub),
                StoredSerializedMessage {
                    header, message, ..
                },
            ) => {
                stub.handle_message(header, message.into_inner().0);
            }
            (
                PeerOutgoingConnection::OutgoingStub(stub),
                StoredSerializedMessage {
                    header, message, ..
                },
            ) => {
                let (msg, buf) = message.into_inner();

                let module = msg.get_module();

                let wire_msg = WireMessage::from_parts(header, module, buf).unwrap();

                dispatch_message(stub, wire_msg, self.to);
            }
        }
    }
}

fn dispatch_message<CN>(stub: CN, message: WireMessage, _to: NodeId)
where
    CN: ByteNetworkStub + 'static,
{
    stub.dispatch_blocking(message)
        .expect("Failed to send message to node");

    //dispatch_message_circuit(stub, message, to, 0);
}

const CIRCUIT_BREAKER_ATTEMPTS: usize = 5;

fn dispatch_message_circuit<CN>(stub: CN, message: WireMessage, to: NodeId, circuits: usize)
where
    CN: ByteNetworkStub + 'static,
{
    if let Err(dispatch_error) = stub.dispatch_message(message) {
        if dispatch_error.can_retry() {
            let message = dispatch_error.into();

            if circuits > CIRCUIT_BREAKER_ATTEMPTS {
                error!(
                    "Failed to send message to node {:?} after {} attempts, blocking",
                    to, circuits
                );

                //TODO: If there are byzantine replicas reading requests slowly,
                // this can start to bottleneck the system. We should consider
                // throwing messages away if they are not delivered after a certain
                // number of attempts.
                stub.dispatch_blocking(message)
                    .expect("Failed to send message to node");

                return;
            }

            warn!("Failed to send message to node {:?} immediately, retrying for the {} time with a circuit breaker pattern", to, circuits);

            handle_failed_message_delivery_circuit(stub, message, to, circuits + 1);
        } else {
            error!(
                "Internal error while sending message to node {:?}: {:?}",
                to, dispatch_error
            );
        }
    };
}

fn handle_failed_message_delivery_circuit<CN>(
    stub: CN,
    wire_msg: WireMessage,
    to: NodeId,
    circuits_made: usize,
) where
    CN: ByteNetworkStub + 'static,
{
    atlas_common::threadpool::execute(move || {
        dispatch_message_circuit(stub, wire_msg, to, circuits_made);
    });
}

pub fn send_message_to_targets<NI, CN, R, O, S, A, L>(
    conn_manager: &PeerConnectionManager<NI, CN, R, O, S, A, L>,
    shared: Option<&Arc<KeyPair>>,
    rng: &Arc<ThreadSafePrng>,
    message: ModMessageWrapped<R, O, S, A>,
    targets: impl Iterator<Item = NodeId>,
) where
    CN: ByteNetworkStub + 'static,
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    L: LookupTable<R, O, S, A>,
    NI: NetworkInformationProvider,
{
    /*if shared.is_none() {
        trace!(
            "Sending message from module {:?} without authentication",
            message.get_module()
        );
    }*/

    let (send_to_me, send_tos) = SendTo::initialize_send_tos(conn_manager, shared, rng, targets);

    let alloc_time = Instant::now();

    atlas_common::threadpool::execute(move || {
        metric_duration(THREADPOOL_PASS_TIME_ID, alloc_time.elapsed());

        let serialize_time_start = Instant::now();

        let mut buf = Vec::new();

        let message_module = message.get_module();

        let digest = match &message {
            ModMessageWrapped::Reconfiguration(reconf) => {
                serialization::serialize_digest::<Vec<u8>, R>(reconf, &mut buf)
            }
            ModMessageWrapped::Protocol(protocol) => {
                serialization::serialize_digest::<Vec<u8>, O>(protocol, &mut buf)
            }
            ModMessageWrapped::StateProtocol(state) => {
                serialization::serialize_digest::<Vec<u8>, S>(state, &mut buf)
            }
            ModMessageWrapped::Application(application) => {
                serialization::serialize_digest::<Vec<u8>, A>(application, &mut buf)
            }
        };

        let digest = quiet_unwrap!(digest);

        let buf = Bytes::from(buf);

        metric_store_count_max(OUTGOING_MESSAGE_SIZE_ID, buf.len() + Header::LENGTH);

        if let Some(send_to_me) = send_to_me {
            send_to_me.value(
                Either::Left((message, buf.clone(), digest)),
                serialize_time_start,
            );
        }

        if let Some(send_tos) = send_tos {
            send_tos.into_iter().for_each(|send_to| {
                if send_to.authenticated_state() {
                    send_to.value(
                        Either::Right((message_module.clone(), buf.clone(), digest)),
                        serialize_time_start,
                    );
                } else {
                    match &message_module {
                        MessageModule::Reconfiguration => {
                            send_to.value(
                                Either::Right((message_module.clone(), buf.clone(), digest)),
                                serialize_time_start,
                            );
                        }
                        _ => {
                            error!(
                                "Attempted to send message to node {:?} while unauthenticated",
                                send_to.to()
                            );
                        }
                    }
                }
            });
        }
    });
}

pub fn send_serialized_message_to_target<NI, CN, R, O, S, A, L>(
    conn_manager: &PeerConnectionManager<NI, CN, R, O, S, A, L>,
    message: StoredSerializedMessage<ModMessageWrapped<R, O, S, A>>,
    target: NodeId,
) where
    CN: ByteNetworkStub + 'static,
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    L: LookupTable<R, O, S, A>,
    NI: NetworkInformationProvider,
{
    let (send_to_me, send_tos) =
        SendTo::initialize_send_tos_serialized(conn_manager, None, conn_manager.rng(), target);

    atlas_common::threadpool::execute(move || {
        if let Some(send_to_me) = send_to_me {
            send_to_me.value_ser(message);
        } else if let Some(send_to) = send_tos {
            if send_to.authenticated_state() {
                send_to.value_ser(message)
            } else {
                match &message.message().original().get_module() {
                    MessageModule::Reconfiguration => send_to.value_ser(message),
                    _ => {
                        error!(
                            "Attempted to send message to node {:?} while unauthenticated",
                            send_to.to()
                        );
                    }
                }
            }
        }
    });
}
