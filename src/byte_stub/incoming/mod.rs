use crate::byte_stub::incoming::pooled_stub::{ConnectedPeersGroup, PooledStubOutput};
use crate::byte_stub::peer_conn_manager::ModuleStubEndPoint;
use crate::byte_stub::peer_conn_manager::PeerStubEndpoints;
use crate::byte_stub::stub_endpoint::StubEndpoint;
use crate::byte_stub::{from_arr, BlankError, NodeIncomingStub};

use crate::config::{ClientPoolConfig, UnpooledConnection};
use crate::lookup_table::{LookupTable, MessageInputStubs, MessageModule};
use crate::message::{Header, StoredMessage, WireMessage};
use crate::reconfiguration::NetworkInformationProvider;
use crate::serialization::Serializable;
use crate::{lookup_table, message_ingestion};

use atlas_common::error::*;
use atlas_common::node_id::{NodeId, NodeType};
use atlas_common::{channel, quiet_unwrap};
use enum_map::EnumMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use strum::IntoEnumIterator;
use tracing::{error, info};

pub(crate) mod pooled_stub;
pub(crate) mod unpooled_stub;

const MODULES: usize = enum_map::enum_len::<MessageModule>();

/// The stub controller for a given peer
/// Maps all message modules to their corresponding controller
pub(crate) struct PeerStubController<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    stub_controller_map: EnumMap<MessageModule, PerMessageModStubController<R, O, S, A>>,
}

/// Again, similarly to [MessageInputStubs], this enum is a necessary evil due to the type system.
///
/// Handle the distribution of stubs for a given message module.
pub enum PerMessageModStubController<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    Reconfiguration(PeerStubControllers<R::Message>),
    Protocol(PeerStubControllers<O::Message>),
    StateProtocol(PeerStubControllers<S::Message>),
    Application(PeerStubControllers<A::Message>),
}

/// The possible controllers for stubs
pub enum PeerStubControllers<M>
where
    M: Send,
{
    Unpooled(unpooled_stub::UnpooledStubManagement<StoredMessage<M>>),
    Pooled(Arc<pooled_stub::ConnectedPeersGroup<StoredMessage<M>>>),
}

/// The input lookup table for a given peer
pub struct PeerStubLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    lookup_table: Arc<EnumMap<MessageModule, MessageInputStubs<R, O, S, A>>>,
}

/// A single incoming connection to a given peer
pub struct PeerIncomingConnection<R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    authenticated: Arc<AtomicBool>,
    lookup_table: L,
    stub_lookup: PeerStubLookupTable<R, O, S, A>,
}

/// The stub for a given peer
pub enum InternalStubTX<M>
where
    M: Send,
{
    Unpooled(unpooled_stub::UnpooledStubTX<StoredMessage<M>>),
    Pooled(pooled_stub::ClientPeer<StoredMessage<M>>),
}

impl<M> InternalStubTX<M>
where
    M: Send,
{
    pub fn handle_message(&self, header: Header, message: M) -> Result<()> {
        match self {
            InternalStubTX::Unpooled(tx) => {
                tx.send(StoredMessage::new(header, message))?;
            }
            InternalStubTX::Pooled(tx) => {
                tx.push_request(StoredMessage::new(header, message))?;
            }
        }

        Ok(())
    }
}

impl<R, O, S, A, L> PeerIncomingConnection<R, O, S, A, L>
where
    L: LookupTable<R, O, S, A>,
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub fn initialize_incoming_conn(
        authenticated: Arc<AtomicBool>,
        lookup_table: L,
        peer_stub: PeerStubLookupTable<R, O, S, A>,
    ) -> Self {
        Self {
            authenticated,
            lookup_table,
            stub_lookup: peer_stub,
        }
    }
}

/// Implementation of the function that handles the message coming from the byte layer
/// and pushes it to the appropriate stub
impl<R, O, S, A, L> NodeIncomingStub for PeerIncomingConnection<R, O, S, A, L>
where
    L: LookupTable<R, O, S, A> + 'static,
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
{
    type Error = BlankError;

    fn handle_message<NI>(
        &self,
        _network_info: &Arc<NI>,
        message: WireMessage,
    ) -> std::result::Result<(), Self::Error>
    where
        NI: NetworkInformationProvider + 'static,
    {
        let peer_stub_lookup = self.stub_lookup.clone();
        //let lookup_table = self.lookup_table.clone();
        //let network_info = network_info.clone();
        let authenticated = self.authenticated.load(Ordering::Relaxed);

        atlas_common::threadpool::execute(move || {
            quiet_unwrap!(message_ingestion::process_wire_message_message(
                message,
                authenticated,
                //&*network_info,
                //&lookup_table,
                &peer_stub_lookup
            ));
        });

        Ok(())
    }
}

impl<R, O, S, A> From<EnumMap<MessageModule, MessageInputStubs<R, O, S, A>>>
    for PeerStubLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn from(lookup_table: EnumMap<MessageModule, MessageInputStubs<R, O, S, A>>) -> Self {
        Self {
            lookup_table: Arc::new(lookup_table),
        }
    }
}

impl<R, O, S, A> lookup_table::PeerStubLookupTable<R, O, S, A> for PeerStubLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn get_stub_for_message(&self, module: &MessageModule) -> &MessageInputStubs<R, O, S, A> {
        &self.lookup_table[module.clone()]
    }
}

impl<R, O, S, A> PeerStubController<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub fn initialize_controller(
        my_id: NodeId,
        node_type: NodeType,
    ) -> (Self, PeerStubEndpoints<R, O, S, A>) {
        let mut controllers = Vec::new();
        let mut stub_output = Vec::new();

        for message_mod in MessageModule::iter() {
            let (controller, output) = match message_mod {
                MessageModule::Reconfiguration => {
                    let (controller, output) =
                        generate_stub_controller_for::<R::Message>(my_id, node_type, message_mod);

                    (
                        PerMessageModStubController::Reconfiguration(controller),
                        ModuleStubEndPoint::Reconfiguration(output),
                    )
                }
                MessageModule::Protocol => {
                    let (controller, output) =
                        generate_stub_controller_for::<O::Message>(my_id, node_type, message_mod);

                    (
                        PerMessageModStubController::Protocol(controller),
                        ModuleStubEndPoint::Protocol(output),
                    )
                }
                MessageModule::StateProtocol => {
                    let (controller, output) =
                        generate_stub_controller_for::<S::Message>(my_id, node_type, message_mod);

                    (
                        PerMessageModStubController::StateProtocol(controller),
                        ModuleStubEndPoint::StateProtocol(output),
                    )
                }
                MessageModule::Application => {
                    let (controller, output) =
                        generate_stub_controller_for::<A::Message>(my_id, node_type, message_mod);

                    (
                        PerMessageModStubController::Application(controller),
                        ModuleStubEndPoint::Application(output),
                    )
                }
            };

            controllers.push(controller);
            stub_output.push(output);
        }

        let map = EnumMap::from_array(from_arr::<_, MODULES>(controllers).unwrap());
        let output_map = EnumMap::from_array(from_arr::<_, MODULES>(stub_output).unwrap());

        (
            Self {
                stub_controller_map: map,
            },
            PeerStubEndpoints {
                stub_output_map: output_map,
            },
        )
    }

    pub(super) fn get_stub_controller_for(
        &self,
        module: &MessageModule,
    ) -> &PerMessageModStubController<R, O, S, A> {
        &self.stub_controller_map[module.clone()]
    }
}

impl<M> PeerStubControllers<M>
where
    M: Send + 'static,
{
    fn initialize_stub_for(&self, node: NodeId) -> InternalStubTX<M> {
        match self {
            PeerStubControllers::Unpooled(unpooled_stub_controller) => {
                InternalStubTX::Unpooled(unpooled_stub_controller.gen_stub_stub_for_peer(node))
            }
            PeerStubControllers::Pooled(pooled) => InternalStubTX::Pooled(pooled.init_client(node)),
        }
    }
}

impl<R, O, S, A> PerMessageModStubController<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub fn initialize_stud_for(&self, node: NodeId) -> MessageInputStubs<R, O, S, A> {
        match self {
            PerMessageModStubController::Reconfiguration(controller) => {
                MessageInputStubs::Reconfiguration(controller.initialize_stub_for(node))
            }
            PerMessageModStubController::Protocol(controller) => {
                MessageInputStubs::Protocol(controller.initialize_stub_for(node))
            }
            PerMessageModStubController::StateProtocol(controller) => {
                MessageInputStubs::StateProtocol(controller.initialize_stub_for(node))
            }
            PerMessageModStubController::Application(controller) => {
                MessageInputStubs::Application(controller.initialize_stub_for(node))
            }
        }
    }
}

// Clone implementation for PeerIncomingConnection
impl<R, O, S, A, L> Clone for PeerIncomingConnection<R, O, S, A, L>
where
    L: Clone,
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn clone(&self) -> Self {
        Self {
            authenticated: self.authenticated.clone(),
            lookup_table: self.lookup_table.clone(),
            stub_lookup: self.stub_lookup.clone(),
        }
    }
}

//Clone for Peer Stub lookup table
impl<R, O, S, A> Clone for PeerStubLookupTable<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn clone(&self) -> Self {
        Self {
            lookup_table: self.lookup_table.clone(),
        }
    }
}

// Clone implementation for InternalStubTX
impl<M> Clone for InternalStubTX<M>
where
    M: Send,
{
    fn clone(&self) -> Self {
        match self {
            InternalStubTX::Unpooled(tx) => InternalStubTX::Unpooled(tx.clone()),
            InternalStubTX::Pooled(tx) => InternalStubTX::Pooled(tx.clone()),
        }
    }
}

/// Generate a stub controller for a given message module.
/// This yields a new stub controller and the corresponding end point where
/// modules can then receive their respective messages
fn generate_stub_controller_for<M>(
    my_id: NodeId,
    my_node_type: NodeType,
    message_mod: MessageModule,
) -> (PeerStubControllers<M>, StubEndpoint<M>)
where
    M: Send + 'static,
{
    match my_node_type {
        NodeType::Replica => match message_mod {
            MessageModule::Reconfiguration
            | MessageModule::Protocol
            | MessageModule::StateProtocol => {
                let (unpooled_stub, rx) =
                    unpooled_stub::UnpooledStubManagement::initialize_controller(
                        Default::default(),
                        message_mod,
                    );

                let peer_stub_controller = PeerStubControllers::Unpooled(unpooled_stub);
                let stub_output = StubEndpoint::Unpooled(rx);

                (peer_stub_controller, stub_output)
            }
            MessageModule::Application => {
                let config = ClientPoolConfig::default();

                let (tx, rx) = channel::sync::new_unbounded_sync(Some(format!(
                    "Pooled stub {message_mod:?} (Incoming)"
                )));

                let stub_control = ConnectedPeersGroup::new(config, tx, rx.clone(), my_id);

                let peer_stub_control = PeerStubControllers::Pooled(stub_control);

                (
                    peer_stub_control,
                    StubEndpoint::Pooled(PooledStubOutput::from(rx)),
                )
            }
        },
        NodeType::Client => {
            let config = UnpooledConnection::new(1024);

            info!(
                "Initializing stub controller for client node with config {:?}",
                config
            );

            // When we are clients we use all unpooled stubs (since we don't have to handle a lot of throughput)
            match message_mod {
                MessageModule::Reconfiguration
                | MessageModule::Protocol
                | MessageModule::StateProtocol
                | MessageModule::Application => {
                    //TODO: We should receive (maybe individual?) configs as arguments, not use the default
                    let (unpooled_stub, rx) =
                        unpooled_stub::UnpooledStubManagement::initialize_controller(
                            config,
                            message_mod,
                        );

                    let peer_stub_controller = PeerStubControllers::Unpooled(unpooled_stub);
                    let stub_output = StubEndpoint::Unpooled(rx);

                    (peer_stub_controller, stub_output)
                }
            }
        }
    }
}
