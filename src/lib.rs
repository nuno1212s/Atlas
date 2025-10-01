#![allow(incomplete_features)]
#![feature(inherent_associated_types)]
#![feature(associated_type_defaults)]
#![allow(dead_code)]

use std::sync::Arc;

use getset::{CopyGetters, Getters};

use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::prng::ThreadSafePrng;

use crate::byte_stub::incoming::PeerIncomingConnection;
use crate::byte_stub::peer_conn_manager::PeerConnectionManager;
use crate::byte_stub::{ByteNetworkController, ByteNetworkControllerInit, ByteNetworkStub};
use crate::lookup_table::EnumLookupTable;
use crate::network_information::initialize_network_info_handle;
use crate::reconfiguration::{NetworkInformationProvider, NetworkReconfigurationCommunication};
use crate::serialization::Serializable;
use crate::stub::{ApplicationStub, OperationStub, ReconfigurationStub, StateProtocolStub};

pub mod byte_stub;
pub mod config;
pub mod lookup_table;
pub mod message;
mod message_ingestion;
mod message_outgoing;
pub mod message_signing;
pub mod metric;
mod network_information;
pub mod reconfiguration;
pub mod serialization;
pub mod stub;

/// The struct that coordinates the entire network stack
/// We have all of the abstractions here, as we want to handle as many
/// possible combinations of implementations as possible
///
/// This module of Atlas is meant to translate our messages into their
/// byte level representation, so they can be safely sent through the
/// underlying network implementation, effectively making this completely
/// abstract on the network type it's running in
#[derive(CopyGetters, Getters)]
pub struct NetworkManagement<NI, CN, BN, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BN: Clone,
{
    // The ID of our node
    #[get_copy = "pub(crate)"]
    id: NodeId,
    // Information about the topology of the network
    #[getset(get = "pub(crate)")]
    network_info: Arc<NI>,
    // The thread safe random number generator
    #[get = "pub(crate)"]
    rng: Arc<ThreadSafePrng>,
    // The controller for all the connections that are incoming into our node
    #[get = "pub(crate)"]
    conn_manager: NodeStubController<NI, CN, R, O, S, A>,
    // The byte level network controller
    #[get = "pub"]
    byte_network_controller: BN,
}

pub type NodeInputStub<R, O, S, A> =
    PeerIncomingConnection<R, O, S, A, EnumLookupTable<R, O, S, A>>;
pub type NodeStubController<NI, CN, R, O, S, A> =
    PeerConnectionManager<NI, CN, R, O, S, A, EnumLookupTable<R, O, S, A>>;

impl<NI, CN, BN, R, O, S, A> NetworkManagement<NI, CN, BN, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    BN: Clone,
    CN: Clone,
{
    type NetworkController = PeerConnectionManager<NI, CN, R, O, S, A, EnumLookupTable<R, O, S, A>>;

    type InputStub = PeerIncomingConnection<R, O, S, A, EnumLookupTable<R, O, S, A>>;

    pub fn initialize(
        network_info: Arc<NI>,
        config: BN::Config,
        reconfiguration_msg: NetworkReconfigurationCommunication,
    ) -> Result<Arc<Self>>
    where
        BN: ByteNetworkControllerInit<
            NI,
            PeerConnectionManager<NI, CN, R, O, S, A, EnumLookupTable<R, O, S, A>>,
            CN,
            PeerIncomingConnection<R, O, S, A, EnumLookupTable<R, O, S, A>>,
        >,
        NI: NetworkInformationProvider + 'static,
        CN: ByteNetworkStub + 'static,
    {
        let own_info = network_info.own_node_info();

        let lookup_table = EnumLookupTable::default();

        let rng = Arc::new(ThreadSafePrng::new());

        let connection_controller = PeerConnectionManager::initialize(
            network_info.clone(),
            own_info.node_id(),
            own_info.node_type(),
            lookup_table,
            rng.clone(),
        );

        // Initialize the thread that will receive the updates from the reconfiguration protocol
        initialize_network_info_handle(reconfiguration_msg, connection_controller.clone());

        // Initialize the underlying byte level network controller
        //TODO: Remove this unwrap
        let network_controller =
            BN::initialize_controller(network_info.clone(), config, connection_controller.clone())
                .unwrap();

        Ok(Arc::new(Self {
            id: own_info.node_id(),
            network_info,
            rng,
            conn_manager: connection_controller,
            byte_network_controller: network_controller,
        }))
    }

    pub fn init_op_stub(&self) -> OperationStub<NI, CN, BN::ConnectionController, R, O, S, A>
    where
        BN: ByteNetworkController,
    {
        OperationStub::new(
            self,
            self.byte_network_controller()
                .connection_controller()
                .clone(),
        )
    }

    pub fn init_reconf_stub(
        &self,
    ) -> ReconfigurationStub<NI, CN, BN::ConnectionController, R, O, S, A>
    where
        BN: ByteNetworkController,
    {
        ReconfigurationStub::new(
            self,
            self.byte_network_controller()
                .connection_controller()
                .clone(),
        )
    }

    pub fn init_state_stub(&self) -> StateProtocolStub<NI, CN, BN::ConnectionController, R, O, S, A>
    where
        BN: ByteNetworkController,
    {
        StateProtocolStub::new(
            self,
            self.byte_network_controller()
                .connection_controller()
                .clone(),
        )
    }

    pub fn init_app_stub(&self) -> ApplicationStub<NI, CN, BN::ConnectionController, R, O, S, A>
    where
        BN: ByteNetworkController,
    {
        ApplicationStub::new(
            self,
            self.byte_network_controller()
                .connection_controller()
                .clone(),
        )
    }
}

impl<NI, CN, BN, R, O, S, A> Clone for NetworkManagement<NI, CN, BN, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    BN: Clone,
    CN: Clone,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            network_info: self.network_info.clone(),
            rng: self.rng.clone(),
            conn_manager: self.conn_manager.clone(),
            byte_network_controller: self.byte_network_controller.clone(),
        }
    }
}
