use crate::byte_stub::connections::active_connections::ActiveConnections;
use crate::byte_stub::incoming::{PeerIncomingConnection, PeerStubController, PeerStubLookupTable};
use crate::byte_stub::outgoing::loopback::LoopbackOutgoingStub;
use crate::byte_stub::outgoing::PeerOutgoingConnection;
use crate::byte_stub::peer_conn::PeerConnection;
use crate::byte_stub::stub_endpoint::StubEndpoint;
use crate::byte_stub::{from_arr, BlankError, ByteNetworkStub, NodeStubController};
use crate::lookup_table::{LookupTable, MessageModule};
use crate::network_information::PendingConnectionManagement;
use crate::reconfiguration::NetworkInformationProvider;
use crate::serialization::Serializable;
use atlas_common::crypto::signature::PublicKey;
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::prng::ThreadSafePrng;
use enum_map::EnumMap;
use getset::CopyGetters;
use getset::Getters;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use strum::IntoEnumIterator;
use tracing::info;

/// The map of endpoints, connecting each MessageModule with the given
/// receiver for that module
pub struct PeerStubEndpoints<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub(crate) stub_output_map: EnumMap<MessageModule, ModuleStubEndPoint<R, O, S, A>>,
}

impl<R, O, S, A> PeerStubEndpoints<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub fn get_endpoint_for_module(
        &self,
        module: &MessageModule,
    ) -> &ModuleStubEndPoint<R, O, S, A> {
        &self.stub_output_map[module.clone()]
    }
}

/// The various possible receivers for a given message module
/// These have to be differentiated because of the implementations for Module Input, namely
/// [BatchedModuleIncomingStub] and [ModuleIncomingStub]
///
/// This is what is going to be used by the final message stub modules in order to receive the
/// processed messages
pub enum ModuleStubEndPoint<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    Reconfiguration(StubEndpoint<R::Message>),
    Protocol(StubEndpoint<O::Message>),
    StateProtocol(StubEndpoint<S::Message>),
    Application(StubEndpoint<A::Message>),
}

impl<R, O, S, A> ModuleStubEndPoint<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    // Unwrapping into a reconfiguration endpoint
    pub fn into_reconfig_endpoint(self) -> StubEndpoint<R::Message> {
        match self {
            ModuleStubEndPoint::Reconfiguration(endpoint) => endpoint,
            _ => unreachable!("Unpacked a wrong endpoint?"),
        }
    }

    // Unwrapping into a protocol endpoint
    pub fn into_protocol_endpoint(self) -> StubEndpoint<O::Message> {
        match self {
            ModuleStubEndPoint::Protocol(endpoint) => endpoint,
            _ => unreachable!("Unpacked a wrong endpoint?"),
        }
    }

    // Unwrapping into a state protocol endpoint
    pub fn into_state_protocol_endpoint(self) -> StubEndpoint<S::Message> {
        match self {
            ModuleStubEndPoint::StateProtocol(endpoint) => endpoint,
            _ => unreachable!("Unpacked a wrong endpoint?"),
        }
    }

    // Unwrapping into an application endpoint
    pub fn into_application_endpoint(self) -> StubEndpoint<A::Message> {
        match self {
            ModuleStubEndPoint::Application(endpoint) => endpoint,
            _ => unreachable!("Unpacked a wrong endpoint?"),
        }
    }
}

impl<R, O, S, A> Clone for ModuleStubEndPoint<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn clone(&self) -> Self {
        match self {
            ModuleStubEndPoint::Reconfiguration(endpoint) => {
                ModuleStubEndPoint::Reconfiguration(endpoint.clone())
            }
            ModuleStubEndPoint::Protocol(endpoint) => {
                ModuleStubEndPoint::Protocol(endpoint.clone())
            }
            ModuleStubEndPoint::StateProtocol(endpoint) => {
                ModuleStubEndPoint::StateProtocol(endpoint.clone())
            }
            ModuleStubEndPoint::Application(endpoint) => {
                ModuleStubEndPoint::Application(endpoint.clone())
            }
        }
    }
}

/// The peer connection manager, responsible for generating and managing connections for a given peer
///
/// Handles all connections , outgoing and incoming
#[derive(Getters, CopyGetters)]
pub struct PeerConnectionManager<NI, CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    // The id of the node this connection manager is responsible for
    #[get_copy = "pub"]
    node_id: NodeId,
    // The look up table, common to all peers, based on the lookup table abstraction
    #[get = "pub"]
    lookup_table: L,
    #[get = "pub"]
    rng: Arc<ThreadSafePrng>,
    network_info: Arc<NI>,
    controller: Arc<PeerStubController<R, O, S, A>>,
    #[get = "pub"]
    endpoints: Arc<PeerStubEndpoints<R, O, S, A>>,
    #[get = ""]
    connections: Arc<ActiveConnections<CN, R, O, S, A, L>>,
}

impl<NI, CN, R, O, S, A, L> PeerConnectionManager<NI, CN, R, O, S, A, L>
where
    L: Send + Clone,
    CN: Clone,
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    NI: NetworkInformationProvider,
{
    pub fn initialize(
        network_info: Arc<NI>,
        id: NodeId,
        node_type: NodeType,
        lookup_table: L,
        rng: Arc<ThreadSafePrng>,
    ) -> Self
    where
        L: LookupTable<R, O, S, A>,
    {
        let (stub_controller, endpoints) = PeerStubController::initialize_controller(id, node_type);

        let loopback = Self::initialize_loopback(&stub_controller, lookup_table.clone(), id);

        let active_conns = ActiveConnections::init(id, loopback);

        Self {
            node_id: id,
            lookup_table,
            rng,
            network_info,
            controller: Arc::new(stub_controller),
            endpoints: Arc::new(endpoints),
            connections: Arc::new(active_conns),
        }
    }

    fn initialize_loopback(
        controller: &PeerStubController<R, O, S, A>,
        l_table: L,
        node: NodeId,
    ) -> PeerConnection<CN, R, O, S, A, L>
    where
        L: LookupTable<R, O, S, A>,
    {
        let table = Self::initialize_stub_lookup_table(controller, node);

        let loopback =
            PeerOutgoingConnection::LoopbackStub(LoopbackOutgoingStub::init(table.clone()));

        let authenticated = Arc::new(AtomicBool::new(true));

        PeerConnection {
            authenticated: authenticated.clone(),
            incoming_connection: PeerIncomingConnection::initialize_incoming_conn(
                authenticated,
                l_table,
                table,
            ),
            outgoing_connection: loopback,
        }
    }

    fn initialize_stub_lookup_table(
        controller: &PeerStubController<R, O, S, A>,
        node_id: NodeId,
    ) -> PeerStubLookupTable<R, O, S, A> {
        // Initialize all of the stubs required to create a new connection
        let mut enum_array = Vec::with_capacity(enum_map::enum_len::<MessageModule>());

        // Initialize all of the stubs required to create a new connection
        // TODO: In the future, maybe add an ability to specify which stubs we want to initialize or not
        for module in MessageModule::iter() {
            let controller = controller.get_stub_controller_for(&module);

            enum_array.push(controller.initialize_stud_for(node_id));
        }

        // SAFETY: We know that the length of the array is correct because we initialized it with the correct capacity
        // And it is checked by a unit test in the test module
        EnumMap::from_array(from_arr::<_, { crate::byte_stub::MODULES }>(enum_array).unwrap())
            .into()
    }

    /// Initialize an incoming connection for a given node
    /// This will initialize all the stubs required to handle messages from that node
    pub fn initialize_incoming_connection_for(
        &self,
        authenticated: Arc<AtomicBool>,
        node_id: NodeId,
    ) -> PeerIncomingConnection<R, O, S, A, L>
    where
        L: LookupTable<R, O, S, A>,
    {
        let lookup_table = self.lookup_table.clone();

        PeerIncomingConnection::initialize_incoming_conn(
            authenticated,
            lookup_table,
            Self::initialize_stub_lookup_table(&self.controller, node_id),
        )
    }

    /// Initialize an outgoing connection for a given node, given a byte level stub to that
    /// node (which is connection oriented)
    pub fn initialize_outgoing_connection_for(
        &self,
        _node_id: NodeId,
        node_stub: CN,
    ) -> PeerOutgoingConnection<CN, R, O, S, A> {
        PeerOutgoingConnection::OutgoingStub(node_stub)
    }

    /// Initialize a connection to a given peer
    pub fn initialize_connection(
        &self,
        node: NodeId,
        node_stub: CN,
    ) -> PeerConnection<CN, R, O, S, A, L>
    where
        L: LookupTable<R, O, S, A>,
    {
        let is_known = self.network_info.get_node_info(&node).is_some();

        let authenticated = Arc::new(AtomicBool::new(is_known));

        info!(
            "Initializing connection to node: {:?} with authenticated {:?}",
            node, authenticated
        );

        PeerConnection {
            authenticated: authenticated.clone(),
            incoming_connection: self.initialize_incoming_connection_for(authenticated, node),
            outgoing_connection: self.initialize_outgoing_connection_for(node, node_stub),
        }
    }

    pub fn get_connection_to_node(
        &self,
        node: &NodeId,
    ) -> Option<PeerConnection<CN, R, O, S, A, L>> {
        self.connections.get_connection(node)
    }
}

impl<NI, CN, R, O, S, A, L> PendingConnectionManagement
    for PeerConnectionManager<NI, CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    L: LookupTable<R, O, S, A>,
    NI: NetworkInformationProvider,
    CN: Send + Clone + Sync,
{
    fn has_pending_connection(&self, node: &NodeId) -> bool {
        self.connections.has_connection(node)
    }

    fn upgrade_connection_to_known(
        &self,
        node: &NodeId,
        _node_type: NodeType,
        _key: PublicKey,
    ) -> atlas_common::error::Result<()> {
        if let Some(c) = self.connections().get_connection(node) {
            c.authenticated
                .store(true, std::sync::atomic::Ordering::Relaxed)
        }

        Ok(())
    }
}

impl<NI, CN, R, O, S, A, L> NodeStubController<CN, PeerIncomingConnection<R, O, S, A, L>>
    for PeerConnectionManager<NI, CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    L: LookupTable<R, O, S, A>,
    NI: NetworkInformationProvider,
    CN: Send + Clone + Sync,
{
    type Error = BlankError;

    fn has_stub_for(&self, node: &NodeId) -> bool {
        self.connections.has_connection(node) || *node == self.node_id
    }

    fn generate_stub_for(
        &self,
        node: NodeId,
        byte_stub: CN,
    ) -> Result<PeerIncomingConnection<R, O, S, A, L>, Self::Error>
    where
        CN: ByteNetworkStub,
    {
        let connection = self.initialize_connection(node, byte_stub);

        let incoming_conn = connection.incoming_connection.clone();

        self.connections.add_connection(node, connection);

        Ok(incoming_conn)
    }

    fn get_stub_for(&self, node: &NodeId) -> Option<PeerIncomingConnection<R, O, S, A, L>> {
        self.connections
            .get_connection(node)
            .map(|c| c.incoming_connection)
    }

    fn shutdown_stubs_for(&self, node: &NodeId) -> Result<(), Self::Error> {
        self.connections.remove_connection(node);

        Ok(())
    }
}

impl<NI, CN, R, O, S, A, L> Clone for PeerConnectionManager<NI, CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    L: Clone,
    CN: Clone,
{
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id,
            lookup_table: self.lookup_table.clone(),
            rng: self.rng.clone(),
            network_info: self.network_info.clone(),
            controller: self.controller.clone(),
            endpoints: self.endpoints.clone(),
            connections: self.connections.clone(),
        }
    }
}
