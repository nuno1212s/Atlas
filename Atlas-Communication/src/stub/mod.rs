use std::collections::BTreeMap;

use crate::byte_stub::connections::NetworkConnectionController;
use atlas_common::channel::sync::ChannelSyncRx;
use std::sync::Arc;
use std::time::Duration;

use crate::byte_stub::peer_conn_manager::PeerConnectionManager;
use crate::byte_stub::stub_endpoint::StubEndpoint;
use crate::byte_stub::ByteNetworkStub;
use crate::lookup_table::{EnumLookupTable, MessageModule};
use crate::message::{SerializedMessage, StoredMessage, StoredSerializedMessage};
use crate::reconfiguration::NetworkInformationProvider;
use crate::serialization::Serializable;
use crate::NetworkManagement;
use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::prng::ThreadSafePrng;

mod incoming;
pub(crate) mod outgoing;

/// The outgoing stub trait, valid for any type of messages
pub trait ModuleOutgoingStub<M> {
    /// Sends a message to a given target.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send(&self, message: M, target: NodeId, flush: bool) -> Result<()>;

    /// Sends a signed message to a given target
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send_signed(&self, message: M, target: NodeId, flush: bool) -> Result<()>;

    /// Broadcast a message to all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast(
        &self,
        message: M,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;

    /// Broadcast a signed message for all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_signed(
        &self,
        message: M,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;

    /// Serialize a message to a given target.
    /// Creates the serialized byte buffer along with the header, so we can send it later.
    fn serialize_digest_message(&self, message: M) -> Result<(SerializedMessage<M>, Digest)>;

    /// Broadcast the serialized messages provided.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<M>>,
    ) -> std::result::Result<(), Vec<NodeId>>;
}

/// The trait that directs the ordering of operations
/// for an incoming stub.
///
/// These methods serve to receive messages from the network layer, for a specific module,
/// with the separations described in [MessageModule]
pub trait ModuleIncomingStub<M>:
    Send + Sync + Clone + AsRef<ChannelSyncRx<StoredMessage<M>>>
{
    /// Get the amount of pending requests contained in this stub
    fn pending_rqs(&self) -> usize;

    /// Receive a message from this stub.
    /// Blocks until a message is available
    fn receive_messages(&self) -> Result<StoredMessage<M>>;

    /// Try to receive messages. If no timeout is provided, then
    /// we immediately return whether a message is present or not (with the corresponding [Option] value).
    /// If a timeout is provided, we wait until the provided timeout
    fn try_receive_messages(&self, timeout: Option<Duration>) -> Result<Option<StoredMessage<M>>>;
}

/// The trait that directs the ordering of operations
/// for a batched incoming stub. This is similar to the [ModuleIncomingStub] trait, but
/// this one is used for modules that have a batched message handling (namely [ConnectedPeersGroup] )
pub trait BatchedModuleIncomingStub<M>:
    Send + Sync + Clone + AsRef<ChannelSyncRx<Vec<StoredMessage<M>>>>
{
    /// Receive a batch of messages from this stub.
    /// Blocks until a message is available
    fn receive_messages(&self) -> Result<Vec<StoredMessage<M>>>;

    /// Try to receive messages. If no timeout is provided, then
    /// we immediately return whether a message is present or not (with the corresponding [Option] value).
    /// If a timeout is provided, we wait until the provided timeout
    fn try_receive_messages(
        &self,
        timeout: Option<Duration>,
    ) -> Result<Option<Vec<StoredMessage<M>>>>;
}

/// A basic network stub
pub trait NetworkStub<T>: Send + Sync
where
    T: Serializable,
{
    /// The outgoing message handler.
    type Outgoing: ModuleOutgoingStub<T::Message>;

    /// The controller of the connections
    type Connections: NetworkConnectionController;

    fn id(&self) -> NodeId;

    fn outgoing_stub(&self) -> &Self::Outgoing;

    fn connections(&self) -> &Arc<Self::Connections>;
}

/// An extended network stub, with the incoming request handling stubs as well
pub trait RegularNetworkStub<T>: NetworkStub<T>
where
    T: Serializable,
{
    type Incoming: ModuleIncomingStub<T::Message>;

    fn incoming_stub(&self) -> &Self::Incoming;
}

/// The batched network stub, with the incoming request handling stubs as well
pub trait BatchedNetworkStub<T>: NetworkStub<T>
where
    T: Serializable,
{
    type Incoming: BatchedModuleIncomingStub<T::Message>;

    fn incoming_stub(&self) -> &Self::Incoming;
}

type PeerCNNMng<NI, CN, R, O, S, A> =
    PeerConnectionManager<NI, CN, R, O, S, A, EnumLookupTable<R, O, S, A>>;

/// In reality, this should all be macros, but I'm not into the macro scene
/// And I can't take the time to learn it atm
/// Reconfiguration stub
/// TODO: Remove dependency on the network management, as it introduces un necessary BN generics
pub struct ReconfigurationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
{
    network_info: Arc<NI>,
    conn_manager: PeerCNNMng<NI, CN, R, O, S, A>,
    rng: Arc<ThreadSafePrng>,
    stub_endpoint: StubEndpoint<R::Message>,
    connections: Arc<BNC>,
}

impl<NI, CN, BNC, R, O, S, A> ReconfigurationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
    CN: Clone,
{
    pub fn new<BN>(
        network_management: &NetworkManagement<NI, CN, BN, R, O, S, A>,
        conn_controller: Arc<BNC>,
    ) -> Self
    where
        BN: Clone,
    {
        let end_point = network_management
            .conn_manager()
            .endpoints()
            .get_endpoint_for_module(&MessageModule::Reconfiguration)
            .clone();

        let end_point = end_point.into_reconfig_endpoint();

        Self {
            network_info: network_management.network_info().clone(),
            conn_manager: network_management.conn_manager().clone(),
            rng: network_management.rng().clone(),
            stub_endpoint: end_point,
            connections: conn_controller,
        }
    }
}

impl<NI, CN, BNC, R, O, S, A> Clone for ReconfigurationStub<NI, CN, BNC, R, O, S, A>
where
    A: Serializable,
    O: Serializable,
    R: Serializable,
    S: Serializable,
    BNC: NetworkConnectionController,
    CN: Clone,
{
    fn clone(&self) -> Self {
        Self {
            network_info: self.network_info.clone(),
            conn_manager: self.conn_manager.clone(),
            rng: self.rng.clone(),
            stub_endpoint: self.stub_endpoint.clone(),
            connections: self.connections.clone(),
        }
    }
}

impl<NI, CN, BNC, R, O, S, A> NetworkStub<R> for ReconfigurationStub<NI, CN, BNC, R, O, S, A>
where
    A: Serializable + 'static,
    O: Serializable + 'static,
    R: Serializable + 'static,
    S: Serializable + 'static,
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    type Outgoing = Self;
    type Connections = BNC;

    fn id(&self) -> NodeId {
        self.conn_manager.node_id()
    }

    fn outgoing_stub(&self) -> &Self::Outgoing {
        self
    }

    fn connections(&self) -> &Arc<Self::Connections> {
        &self.connections
    }
}

/// Operation stub
///
///
pub struct OperationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
{
    network_info: Arc<NI>,
    conn_manager: PeerCNNMng<NI, CN, R, O, S, A>,
    rng: Arc<ThreadSafePrng>,
    stub_endpoint: StubEndpoint<O::Message>,
    connections: Arc<BNC>,
}

impl<NI, CN, BNC, R, O, S, A> OperationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
{
    pub fn new<BN>(
        network_management: &NetworkManagement<NI, CN, BN, R, O, S, A>,
        conn_controller: Arc<BNC>,
    ) -> Self
    where
        BN: Clone,
        CN: Clone,
    {
        let end_point = network_management
            .conn_manager()
            .endpoints()
            .get_endpoint_for_module(&MessageModule::Protocol)
            .clone();

        let end_point = end_point.into_protocol_endpoint();

        Self {
            network_info: network_management.network_info().clone(),
            conn_manager: network_management.conn_manager().clone(),
            rng: network_management.rng().clone(),
            stub_endpoint: end_point,
            connections: conn_controller,
        }
    }
}

impl<NI, CN, BNC, R, O, S, A> Clone for OperationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
    CN: Clone,
{
    fn clone(&self) -> Self {
        Self {
            network_info: self.network_info.clone(),
            conn_manager: self.conn_manager.clone(),
            rng: self.rng.clone(),
            stub_endpoint: self.stub_endpoint.clone(),
            connections: self.connections.clone(),
        }
    }
}

impl<NI, CN, BNC, R, O, S, A> NetworkStub<O> for OperationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    CN: ByteNetworkStub + 'static,
    NI: NetworkInformationProvider,
    BNC: NetworkConnectionController,
{
    type Outgoing = Self;
    type Connections = BNC;

    fn id(&self) -> NodeId {
        self.conn_manager.node_id()
    }

    fn outgoing_stub(&self) -> &Self::Outgoing {
        self
    }

    fn connections(&self) -> &Arc<Self::Connections> {
        &self.connections
    }
}

/// State protocol stub
pub struct StateProtocolStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
{
    network_info: Arc<NI>,
    conn_manager: PeerCNNMng<NI, CN, R, O, S, A>,
    rng: Arc<ThreadSafePrng>,
    stub_endpoint: StubEndpoint<S::Message>,
    connections: Arc<BNC>,
}

impl<NI, CN, BNC, R, O, S, A> StateProtocolStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
{
    pub fn new<BN>(
        network_management: &NetworkManagement<NI, CN, BN, R, O, S, A>,
        conn_controller: Arc<BNC>,
    ) -> Self
    where
        BN: Clone,
        CN: Clone,
    {
        let end_point = network_management
            .conn_manager()
            .endpoints()
            .get_endpoint_for_module(&MessageModule::StateProtocol)
            .clone();

        let end_point = end_point.into_state_protocol_endpoint();

        Self {
            network_info: network_management.network_info().clone(),
            conn_manager: network_management.conn_manager().clone(),
            rng: network_management.rng().clone(),
            stub_endpoint: end_point,
            connections: conn_controller,
        }
    }
}

//Clone impl
impl<NI, CN, BNC, R, O, S, A> Clone for StateProtocolStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
    CN: Clone,
{
    fn clone(&self) -> Self {
        Self {
            network_info: self.network_info.clone(),
            conn_manager: self.conn_manager.clone(),
            rng: self.rng.clone(),
            stub_endpoint: self.stub_endpoint.clone(),
            connections: self.connections.clone(),
        }
    }
}

impl<NI, CN, BNC, R, O, S, A> NetworkStub<S> for StateProtocolStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    type Outgoing = Self;
    type Connections = BNC;

    fn id(&self) -> NodeId {
        self.conn_manager.node_id()
    }

    fn outgoing_stub(&self) -> &Self::Outgoing {
        self
    }

    fn connections(&self) -> &Arc<Self::Connections> {
        &self.connections
    }
}

/// Application stub
pub struct ApplicationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
{
    network_info: Arc<NI>,
    conn_manager: PeerCNNMng<NI, CN, R, O, S, A>,
    rng: Arc<ThreadSafePrng>,
    stub_endpoint: StubEndpoint<A::Message>,
    connections: Arc<BNC>,
}

impl<NI, CN, BNC, R, O, S, A> ApplicationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
{
    pub fn new<BN>(
        network_management: &NetworkManagement<NI, CN, BN, R, O, S, A>,
        conn_controller: Arc<BNC>,
    ) -> Self
    where
        BN: Clone,
        CN: Clone,
    {
        let end_point = network_management
            .conn_manager()
            .endpoints()
            .get_endpoint_for_module(&MessageModule::Application)
            .clone();

        let end_point = end_point.into_application_endpoint();

        Self {
            network_info: network_management.network_info().clone(),
            conn_manager: network_management.conn_manager().clone(),
            rng: network_management.rng().clone(),
            stub_endpoint: end_point,
            connections: conn_controller,
        }
    }
}

impl<NI, CN, BNC, R, O, S, A> Clone for ApplicationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BNC: NetworkConnectionController,
    CN: Clone,
{
    fn clone(&self) -> Self {
        Self {
            network_info: self.network_info.clone(),
            conn_manager: self.conn_manager.clone(),
            rng: self.rng.clone(),
            stub_endpoint: self.stub_endpoint.clone(),
            connections: self.connections.clone(),
        }
    }
}

impl<NI, CN, BNC, R, O, S, A> NetworkStub<A> for ApplicationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    type Outgoing = Self;
    type Connections = BNC;

    fn id(&self) -> NodeId {
        self.conn_manager.node_id()
    }

    fn outgoing_stub(&self) -> &Self::Outgoing {
        self
    }

    fn connections(&self) -> &Arc<Self::Connections> {
        &self.connections
    }
}
