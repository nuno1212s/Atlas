#![allow(incomplete_features)]
#![feature(inherent_associated_types)]
extern crate core;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use getset::Getters;

use atlas_common::node_id::NodeId;
use atlas_common::socket;
use atlas_common::socket::SyncListener;
use atlas_communication::byte_stub::{
    ByteNetworkController, ByteNetworkControllerInit, NodeIncomingStub, NodeStubController,
};
use atlas_communication::reconfiguration::NetworkInformationProvider;

use crate::config::MIOConfig;
use crate::conn_util::ConnCounts;
use crate::connections::{ByteMessageSendStub, Connections};
use crate::epoll::{init_worker_group_handle, initialize_worker_group};

pub mod config;
pub(crate) mod conn_util;
mod connections;
mod epoll;
pub mod metrics;

/// The byte level TCP MIO epoll based module
/// Utilizes the MIO library to provide a TCP based communication layer
/// much faster than the existing async std or tokio based options, due to the much lower overhead
/// of (not) changing the context of the execution
///
/// The IS generic type indicates the type for the input stubs
/// The CNP indicates the type for the stub controller of the upper network layer
#[derive(Getters)]
pub struct MIOTCPNode<NI, IS, CNP>
where
    NI: NetworkInformationProvider,
    CNP: Clone,
{
    #[get = "pub"]
    network_information: Arc<NI>,
    #[get = "pub"]
    stub_controller: CNP,
    connections: Arc<Connections<NI, IS, CNP>>,
}

impl<NI, IS, CNP> MIOTCPNode<NI, IS, CNP>
where
    CNP: Clone,
    NI: NetworkInformationProvider,
{
    fn setup_connection(_id: &NodeId, server_addr: &SocketAddr) -> Result<SyncListener, io::Error> {
        socket::bind_sync_server(*server_addr)
    }
}

pub type ByteStubType = ByteMessageSendStub;

impl<NI, IS, CNP> ByteNetworkController for MIOTCPNode<NI, IS, CNP>
where
    NI: NetworkInformationProvider + 'static,
    CNP: NodeStubController<ByteMessageSendStub, IS> + 'static,
    IS: NodeIncomingStub + 'static,
{
    type Config = MIOConfig;

    type ConnectionController = Connections<NI, IS, CNP>;

    /// The controller of the connections
    fn connection_controller(&self) -> &Arc<Self::ConnectionController> {
        &self.connections
    }
}

impl<NI, IS, CNP> ByteNetworkControllerInit<NI, CNP, ByteMessageSendStub, IS>
    for MIOTCPNode<NI, IS, CNP>
where
    NI: NetworkInformationProvider + 'static,
    CNP: NodeStubController<ByteMessageSendStub, IS> + 'static,
    IS: NodeIncomingStub + 'static,
{
    type Error = io::Error;

    fn initialize_controller(
        network_info: Arc<NI>,
        config: Self::Config,
        stub_controllers: CNP,
    ) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        let (handle, receivers) = init_worker_group_handle::<IS>(config.epoll_worker_count());

        let connections = Arc::new(Connections::initialize_connections(
            network_info.clone(),
            handle,
            ConnCounts::from_tcp_config(config.tcp_configs()),
            stub_controllers.clone(),
        ));

        initialize_worker_group(connections.clone(), receivers)?;

        let node_info = network_info.own_node_info();

        if let Some(addr) = config.tcp_configs().bind_addrs() {
            addr.iter().try_for_each(|addr| {
                let listener = Self::setup_connection(&node_info.node_id(), addr)?;

                connections.setup_tcp_worker(listener);

                Ok::<(), io::Error>(())
            })?;
        } else {
            let listener = Self::setup_connection(&node_info.node_id(), node_info.addr().socket())?;

            connections.setup_tcp_worker(listener);
        };

        Ok(Self {
            network_information: network_info,
            stub_controller: stub_controllers,
            connections,
        })
    }
}

impl<NI, IS, CNP> Clone for MIOTCPNode<NI, IS, CNP>
where
    NI: NetworkInformationProvider,
    CNP: Clone,
{
    fn clone(&self) -> Self {
        Self {
            network_information: self.network_information.clone(),
            stub_controller: self.stub_controller.clone(),
            connections: self.connections.clone(),
        }
    }
}
