use std::future::Future;
use std::sync::Arc;

use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_communication::byte_stub::incoming::PeerIncomingConnection;
use atlas_communication::byte_stub::peer_conn_manager::PeerConnectionManager;
use atlas_communication::byte_stub::{
    ByteNetworkController, ByteNetworkControllerInit, ByteNetworkStub,
};
use atlas_communication::lookup_table::EnumLookupTable;
use atlas_communication::reconfiguration::{
    NetworkInformationProvider, NetworkReconfigurationCommunication,
};
use atlas_communication::serialization::Serializable;
use atlas_communication::stub::{
    ApplicationStub, NetworkStub, ReconfigurationStub, RegularNetworkStub,
};
use atlas_communication::NetworkManagement;
use atlas_core::serialize::NoProtocol;
use atlas_smr_application::serialize::ApplicationData;

use crate::serialize::SMRSysMsg;

/// The Client node abstractions, different from the replica abstractions
pub trait SMRClientNetworkNode<NI, RM, D>
where
    RM: Serializable,
    D: ApplicationData + 'static,
{
    /// Configuration type for this network node
    type Config;

    /// The app node type
    type AppNode: RegularNetworkStub<SMRSysMsg<D>>;

    /// The reconfiguration node type
    type ReconfigurationNode: RegularNetworkStub<RM>;

    fn id(&self) -> NodeId;

    /// The app node type
    fn app_node(&self) -> &Arc<Self::AppNode>;

    /// The reconfiguration node type
    fn reconfiguration_node(&self) -> &Arc<Self::ReconfigurationNode>;

    /// Bootstrap the node
    fn bootstrap(
        network_info: Arc<NI>,
        config: Self::Config,
        reconf: NetworkReconfigurationCommunication,
    ) -> impl Future<Output = Result<Self>> + Send
    where
        Self: Sized;
}

/// Type alias for the client reconfiguration stub
/// This is just to make the types less cumbersome
/// We use NoProtocol for the ordering and state transfer protocols, as clients do not participate in them
pub type CLIReconfigurationStub<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub,
    BN: ByteNetworkController,
    RM: Serializable,
    D: ApplicationData + 'static,
> = ReconfigurationStub<NI, CN, BN::ConnectionController, RM, NoProtocol, NoProtocol, SMRSysMsg<D>>;

/// Type alias for the client application stub
/// This is just to make the types less cumbersome
/// We use no protocol for the ordering and state transfer protocols, as clients do not participate in them
pub type CLIAppStub<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub,
    BN: ByteNetworkController,
    RM: Serializable,
    D: ApplicationData + 'static,
> = ApplicationStub<NI, CN, BN::ConnectionController, RM, NoProtocol, NoProtocol, SMRSysMsg<D>>;

/// Node wrapper for the client side node.
/// Used to wrap the types and make this into something simple and effective to use
pub struct CLINodeWrapper<CN, BN, NI, RM, D>
where
    NI: NetworkInformationProvider,
    RM: Serializable + 'static,
    CN: ByteNetworkStub + 'static,
    BN: ByteNetworkController,
    D: ApplicationData + 'static,
{
    reconf_stub: Arc<CLIReconfigurationStub<NI, CN, BN, RM, D>>,
    app_stub: Arc<CLIAppStub<NI, CN, BN, RM, D>>,
}

/// Type alias for the peer connection manager used in the client node
/// This is just to make the types less cumbersome
/// We use NoProtocol for the ordering and state transfer protocols, as clients do not participate in them
type CLIPeerCNNMan<NI, CN, RM: Serializable, D: ApplicationData + 'static> = PeerConnectionManager<
    NI,
    CN,
    RM,
    NoProtocol,
    NoProtocol,
    SMRSysMsg<D>,
    EnumLookupTable<RM, NoProtocol, NoProtocol, SMRSysMsg<D>>,
>;

/// Type alias for the peer incoming connection used in the client node
/// This is just to make the types less cumbersome
/// We use NoProtocol for the ordering and state transfer protocols, as clients do not participate in them
type CLIPeerInn<RM: Serializable, D: ApplicationData + 'static> = PeerIncomingConnection<
    RM,
    NoProtocol,
    NoProtocol,
    SMRSysMsg<D>,
    EnumLookupTable<RM, NoProtocol, NoProtocol, SMRSysMsg<D>>,
>;

impl<CN, BN, NI, RM, D> SMRClientNetworkNode<NI, RM, D> for CLINodeWrapper<CN, BN, NI, RM, D>
where
    NI: NetworkInformationProvider + 'static,
    RM: Serializable + 'static,
    D: ApplicationData + 'static,
    CN: ByteNetworkStub + 'static,
    BN: ByteNetworkControllerInit<NI, CLIPeerCNNMan<NI, CN, RM, D>, CN, CLIPeerInn<RM, D>>,
{
    type Config = BN::Config;

    type AppNode = CLIAppStub<NI, CN, BN, RM, D>;

    type ReconfigurationNode = CLIReconfigurationStub<NI, CN, BN, RM, D>;

    fn id(&self) -> NodeId {
        self.reconf_stub.id()
    }

    fn app_node(&self) -> &Arc<Self::AppNode> {
        &self.app_stub
    }

    fn reconfiguration_node(&self) -> &Arc<Self::ReconfigurationNode> {
        &self.reconf_stub
    }

    async fn bootstrap(
        network_info: Arc<NI>,
        config: Self::Config,
        reconf: NetworkReconfigurationCommunication,
    ) -> Result<Self> {
        let cfg = config;

        let arc =
            NetworkManagement::<NI, CN, BN, RM, NoProtocol, NoProtocol, SMRSysMsg<D>>::initialize(
                network_info,
                cfg,
                reconf,
            )?;

        Ok(Self {
            reconf_stub: Arc::new(arc.init_reconf_stub()),
            app_stub: Arc::new(arc.init_app_stub()),
        })
    }
}
