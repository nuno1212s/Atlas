use anyhow::anyhow;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::phantom::FPhantom;
use atlas_communication::byte_stub::incoming::PeerIncomingConnection;
use atlas_communication::byte_stub::peer_conn_manager::PeerConnectionManager;
use atlas_communication::byte_stub::{
    ByteNetworkController, ByteNetworkControllerInit, ByteNetworkStub,
};
use atlas_communication::lookup_table::EnumLookupTable;
use atlas_communication::message::{
    Buf, SerializedMessage, StoredMessage, StoredSerializedMessage,
};
use atlas_communication::reconfiguration::{
    NetworkInformationProvider, NetworkReconfigurationCommunication,
};
use atlas_communication::serialization::Serializable;
use atlas_communication::stub::{
    ApplicationStub, BatchedModuleIncomingStub, BatchedNetworkStub, ModuleOutgoingStub,
    NetworkStub, OperationStub, ReconfigurationStub, RegularNetworkStub, StateProtocolStub,
};
use atlas_communication::NetworkManagement;
use atlas_core::messages::{ForwardedRequestsMessage, RequestMessage};
use atlas_core::ordering_protocol::networking::serialize::{
    OrderingProtocolMessage, ViewTransferProtocolMessage,
};
use atlas_core::ordering_protocol::networking::{
    OrderProtocolSendNode, ViewTransferProtocolSendNode,
};
use atlas_core::request_pre_processing::network::RequestPreProcessingHandle;
use atlas_logging_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_logging_core::log_transfer::networking::LogTransferSendNode;
use atlas_smr_application::serialize::ApplicationData;

use crate::exec::{ReplyNode, RequestType};
use crate::message::{OrderableMessage, SystemMessage};
use crate::serialize::{SMRSysMessage, SMRSysMsg, Service, ServiceMessage, StateSys};
use crate::state_transfer::networking::serialize::StateTransferMessage;
use crate::state_transfer::networking::StateTransferSendNode;
use crate::{SMRReply, SMRReq};

pub mod client;
pub mod signature_ver;

pub type RType<D> = SMRSysMsg<D>;

pub trait SMRReplicaNetworkNode<NI, RM, D, P, L, VT, S>
where
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
    NI: NetworkInformationProvider,
    RM: Serializable,
{
    type Config;

    /// The type that encapsulates the necessary wrapping and unwrapping of the messages
    ///
    type ProtocolNode: OrderProtocolSendNode<SMRReq<D>, P>
        + LogTransferSendNode<SMRReq<D>, P, L>
        + ViewTransferProtocolSendNode<VT>
        + RegularNetworkStub<Service<D, P, L, VT>>;

    type ApplicationNode: RequestPreProcessingHandle<SMRSysMessage<D>> + ReplyNode<SMRReply<D>>;

    type StateTransferNode: StateTransferSendNode<S> + RegularNetworkStub<StateSys<S>>;

    type ReconfigurationNode: RegularNetworkStub<RM>;

    fn id(&self) -> NodeId;

    fn protocol_node(&self) -> &Arc<Self::ProtocolNode>;

    fn app_node(&self) -> &Arc<Self::ApplicationNode>;

    fn state_transfer_node(&self) -> &Arc<Self::StateTransferNode>;

    fn reconfiguration_node(&self) -> &Arc<Self::ReconfigurationNode>;

    fn bootstrap(
        network_info: Arc<NI>,
        config: Self::Config,
        reconf: NetworkReconfigurationCommunication,
    ) -> impl Future<Output = Result<Self>> + Send
    where
        Self: Sized;
}

/// Type alias for the replica operation stub
/// This is just to make the types less cumbersome
pub type RPOperationStub<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub,
    BN: ByteNetworkController,
    RM: Serializable,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = OperationStub<
    NI,
    CN,
    BN::ConnectionController,
    RM,
    Service<D, P, L, VT>,
    StateSys<S>,
    SMRSysMsg<D>,
>;

/// Type alias for the replica protocol node
/// This is just to make the types less cumbersome
pub type RPProtocolNode<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BN: ByteNetworkController,
    RM: Serializable + 'static,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = ProtocolNode<NI, D, P, L, VT, RPOperationStub<NI, CN, BN, RM, D, P, L, VT, S>>;

pub type RPAppStub<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub,
    BN: ByteNetworkController,
    RM: Serializable,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = StateProtocolStub<
    NI,
    CN,
    BN::ConnectionController,
    RM,
    Service<D, P, L, VT>,
    StateSys<S>,
    SMRSysMsg<D>,
>;

/// Type alias for the replica state transfer node
pub type RPStateTransferNode<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BN: ByteNetworkController,
    RM: Serializable + 'static,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = StateTransferNode<S, RPAppStub<NI, CN, BN, RM, D, P, L, VT, S>>;

pub type RPAppState<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BN: ByteNetworkController,
    RM: Serializable + 'static,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = ApplicationStub<
    NI,
    CN,
    BN::ConnectionController,
    RM,
    Service<D, P, L, VT>,
    StateSys<S>,
    SMRSysMsg<D>,
>;

pub type RPAppNode<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BN: ByteNetworkController,
    RM: Serializable + 'static,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = AppNode<D, RPAppState<NI, CN, BN, RM, D, P, L, VT, S>>;

pub type RPReconfNode<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub,
    BN: ByteNetworkController,
    RM: Serializable,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = ReconfigurationStub<
    NI,
    CN,
    BN::ConnectionController,
    RM,
    Service<D, P, L, VT>,
    StateSys<S>,
    SMRSysMsg<D>,
>;

#[allow(clippy::type_complexity)]
pub struct ReplicaNodeWrapper<CN, BN, NI, RM, D, P, L, VT, S>
where
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
    NI: NetworkInformationProvider,
    RM: Serializable + 'static,
    CN: ByteNetworkStub + 'static,
    BN: ByteNetworkController,
{
    op_stub: Arc<RPProtocolNode<NI, CN, BN, RM, D, P, L, VT, S>>,
    state_transfer_stub: Arc<RPStateTransferNode<NI, CN, BN, RM, D, P, L, VT, S>>,
    app_stub: Arc<RPAppNode<NI, CN, BN, RM, D, P, L, VT, S>>,
    reconf_stub: Arc<RPReconfNode<NI, CN, BN, RM, D, P, L, VT, S>>,
}

type PeerCNNMan<
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub,
    RM: Serializable,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = PeerConnectionManager<
    NI,
    CN,
    RM,
    Service<D, P, L, VT>,
    StateSys<S>,
    SMRSysMsg<D>,
    EnumLookupTable<RM, Service<D, P, L, VT>, StateSys<S>, SMRSysMsg<D>>,
>;
type PeerInn<
    RM: Serializable,
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
> = PeerIncomingConnection<
    RM,
    Service<D, P, L, VT>,
    StateSys<S>,
    SMRSysMsg<D>,
    EnumLookupTable<RM, Service<D, P, L, VT>, StateSys<S>, SMRSysMsg<D>>,
>;

impl<CN, BN, NI, RM, D, P, L, VT, S> SMRReplicaNetworkNode<NI, RM, D, P, L, VT, S>
    for ReplicaNodeWrapper<CN, BN, NI, RM, D, P, L, VT, S>
where
    D: ApplicationData + 'static,
    //TODO: Fix this when the lazy_type_alias feature is stable and we can use SMRReq
    P: OrderingProtocolMessage<RequestMessage<D::Request>> + 'static,
    L: LogTransferMessage<RequestMessage<D::Request>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    S: StateTransferMessage + 'static,
    NI: NetworkInformationProvider + 'static,
    RM: Serializable + 'static,
    CN: ByteNetworkStub + 'static,
    BN: ByteNetworkControllerInit<
        NI,
        PeerCNNMan<NI, CN, RM, D, P, L, VT, S>,
        CN,
        PeerInn<RM, D, P, L, VT, S>,
    >,
{
    type Config = BN::Config;

    type ProtocolNode = RPProtocolNode<NI, CN, BN, RM, D, P, L, VT, S>;
    type ApplicationNode = RPAppNode<NI, CN, BN, RM, D, P, L, VT, S>;
    type StateTransferNode = RPStateTransferNode<NI, CN, BN, RM, D, P, L, VT, S>;
    type ReconfigurationNode = RPReconfNode<NI, CN, BN, RM, D, P, L, VT, S>;

    #[inline(always)]
    fn id(&self) -> NodeId {
        self.op_stub.0.id()
    }

    #[inline(always)]
    fn protocol_node(&self) -> &Arc<Self::ProtocolNode> {
        &self.op_stub
    }

    #[inline(always)]
    fn app_node(&self) -> &Arc<Self::ApplicationNode> {
        &self.app_stub
    }

    #[inline(always)]
    fn state_transfer_node(&self) -> &Arc<Self::StateTransferNode> {
        &self.state_transfer_stub
    }

    #[inline(always)]
    fn reconfiguration_node(&self) -> &Arc<Self::ReconfigurationNode> {
        &self.reconf_stub
    }

    async fn bootstrap(
        network_info: Arc<NI>,
        config: Self::Config,
        reconf: NetworkReconfigurationCommunication,
    ) -> Result<Self>
    where
        Self: Sized,
    {
        let cfg = config;

        let network_mngmt = NetworkManagement::<
            NI,
            CN,
            BN,
            RM,
            Service<D, P, L, VT>,
            StateSys<S>,
            SMRSysMsg<D>,
        >::initialize(network_info.clone(), cfg, reconf)?;

        Ok(Self {
            op_stub: Arc::new(ProtocolNode(
                network_mngmt.init_op_stub(),
                network_info,
                Default::default(),
            )),
            state_transfer_stub: Arc::new(StateTransferNode(
                network_mngmt.init_state_stub(),
                Default::default(),
            )),
            app_stub: Arc::new(AppNode(network_mngmt.init_app_stub(), Default::default())),
            reconf_stub: Arc::new(network_mngmt.init_reconf_stub()),
        })
    }
}

pub struct ProtocolNode<NI, D, P, L, VT, NT>(NT, Arc<NI>, FPhantom<(NI, D, P, L, VT)>)
where
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    NT: RegularNetworkStub<Service<D, P, L, VT>>;

pub struct AppNode<D, NT>(pub(crate) NT, FPhantom<D>)
where
    D: ApplicationData + 'static,
    NT: BatchedNetworkStub<SMRSysMsg<D>>;

pub struct StateTransferNode<S, NT>(NT, FPhantom<S>)
where
    S: StateTransferMessage,
    NT: RegularNetworkStub<StateSys<S>>;

impl<NI, D, P, L, VT, NT> NetworkStub<Service<D, P, L, VT>> for ProtocolNode<NI, D, P, L, VT, NT>
where
    D: 'static + ApplicationData,
    L: 'static + LogTransferMessage<SMRReq<D>, P>,
    P: 'static + OrderingProtocolMessage<SMRReq<D>>,
    VT: 'static + ViewTransferProtocolMessage,
    NT: RegularNetworkStub<Service<D, P, L, VT>>,
    NI: NetworkInformationProvider,
{
    type Outgoing = NT::Outgoing;
    type Connections = NT::Connections;

    #[inline(always)]
    fn id(&self) -> NodeId {
        self.0.id()
    }

    #[inline(always)]
    fn outgoing_stub(&self) -> &Self::Outgoing {
        self.0.outgoing_stub()
    }

    #[inline(always)]
    fn connections(&self) -> &Arc<Self::Connections> {
        self.0.connections()
    }
}

impl<NI, D, P, L, VT, NT> RegularNetworkStub<Service<D, P, L, VT>>
    for ProtocolNode<NI, D, P, L, VT, NT>
where
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    NT: RegularNetworkStub<Service<D, P, L, VT>>,
    NI: NetworkInformationProvider,
{
    type Incoming = NT::Incoming;

    #[inline(always)]
    fn incoming_stub(&self) -> &Self::Incoming {
        self.0.incoming_stub()
    }
}

impl<NI, D, P, L, VT, NT> OrderProtocolSendNode<SMRReq<D>, P> for ProtocolNode<NI, D, P, L, VT, NT>
where
    D: ApplicationData,
    P: OrderingProtocolMessage<SMRReq<D>>,
    L: LogTransferMessage<SMRReq<D>, P>,
    VT: ViewTransferProtocolMessage,
    NT: RegularNetworkStub<Service<D, P, L, VT>> + 'static,
    NI: NetworkInformationProvider + 'static,
{
    type NetworkInfoProvider = NI;

    #[inline(always)]
    fn id(&self) -> NodeId {
        self.0.id()
    }

    #[inline(always)]
    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider> {
        &self.1
    }

    #[inline(always)]
    fn forward_requests<I>(
        &self,
        fwd_requests: ForwardedRequestsMessage<SMRReq<D>>,
        targets: I,
    ) -> std::result::Result<(), Vec<NodeId>>
    where
        I: Iterator<Item = NodeId>,
    {
        self.0.outgoing_stub().broadcast_signed(
            SystemMessage::ForwardedRequestMessage(fwd_requests),
            targets,
        )
    }

    #[inline(always)]
    fn send(
        &self,
        message: P::ProtocolMessage,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        self.0
            .outgoing_stub()
            .send(SystemMessage::from_protocol_message(message), target, flush)
    }

    #[inline(always)]
    fn send_signed(
        &self,
        message: P::ProtocolMessage,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        self.0.outgoing_stub().send_signed(
            SystemMessage::from_protocol_message(message),
            target,
            flush,
        )
    }

    #[inline(always)]
    fn broadcast<I>(
        &self,
        message: P::ProtocolMessage,
        targets: I,
    ) -> std::result::Result<(), Vec<NodeId>>
    where
        I: Iterator<Item = NodeId>,
    {
        self.0
            .outgoing_stub()
            .broadcast(SystemMessage::from_protocol_message(message), targets)
    }

    #[inline(always)]
    fn broadcast_signed<I>(
        &self,
        message: P::ProtocolMessage,
        targets: I,
    ) -> std::result::Result<(), Vec<NodeId>>
    where
        I: Iterator<Item = NodeId>,
    {
        self.0
            .outgoing_stub()
            .broadcast_signed(SystemMessage::from_protocol_message(message), targets)
    }

    #[inline(always)]
    fn serialize_digest_message(
        &self,
        message: P::ProtocolMessage,
    ) -> atlas_common::error::Result<(SerializedMessage<P::ProtocolMessage>, Digest)> {
        let (message, digest) = self
            .0
            .outgoing_stub()
            .serialize_digest_message(SystemMessage::from_protocol_message(message))?;

        let (message, bytes): (ServiceMessage<D, P, L, VT>, Buf) = message.into_inner();

        let message = message.into_protocol_message();

        Ok((SerializedMessage::new(message, bytes), digest))
    }

    #[inline(always)]
    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<P::ProtocolMessage>>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let mut map = BTreeMap::new();

        for (node, message) in messages.into_iter() {
            let (header, message) = message.into_inner();

            let (message, bytes) = message.into_inner();

            let sys_msg = SystemMessage::from_protocol_message(message);

            let serialized_msg = SerializedMessage::new(sys_msg, bytes);

            map.insert(node, StoredMessage::new(header, serialized_msg));
        }

        self.0.outgoing_stub().broadcast_serialized(map)
    }
}

impl<NI, D, P, L, VT, NT> LogTransferSendNode<SMRReq<D>, P, L> for ProtocolNode<NI, D, P, L, VT, NT>
where
    D: ApplicationData,
    P: OrderingProtocolMessage<SMRReq<D>>,
    L: LogTransferMessage<SMRReq<D>, P>,
    VT: ViewTransferProtocolMessage,
    NT: RegularNetworkStub<Service<D, P, L, VT>>,
    NI: NetworkInformationProvider,
{
    #[inline(always)]
    fn id(&self) -> NodeId {
        self.0.id()
    }

    #[inline(always)]
    fn send(
        &self,
        message: L::LogTransferMessage,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        self.0.outgoing_stub().send(
            SystemMessage::from_log_transfer_message(message),
            target,
            flush,
        )
    }

    #[inline(always)]
    fn send_signed(
        &self,
        message: L::LogTransferMessage,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        self.0.outgoing_stub().send_signed(
            SystemMessage::from_log_transfer_message(message),
            target,
            flush,
        )
    }

    #[inline(always)]
    fn broadcast(
        &self,
        message: L::LogTransferMessage,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        self.0
            .outgoing_stub()
            .broadcast(SystemMessage::from_log_transfer_message(message), targets)
    }

    #[inline(always)]
    fn broadcast_signed(
        &self,
        message: L::LogTransferMessage,
        target: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        self.0
            .outgoing_stub()
            .broadcast_signed(SystemMessage::from_log_transfer_message(message), target)
    }

    #[inline(always)]
    fn serialize_digest_message(
        &self,
        message: L::LogTransferMessage,
    ) -> atlas_common::error::Result<(SerializedMessage<L::LogTransferMessage>, Digest)> {
        let (message, digest) = self
            .0
            .outgoing_stub()
            .serialize_digest_message(SystemMessage::from_log_transfer_message(message))?;

        let (message, bytes): (ServiceMessage<D, P, L, VT>, Buf) = message.into_inner();

        let message = message.into_log_transfer_message();

        Ok((SerializedMessage::new(message, bytes), digest))
    }

    #[inline(always)]
    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<L::LogTransferMessage>>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let mut map = BTreeMap::new();

        for (node, message) in messages.into_iter() {
            let (header, message) = message.into_inner();

            let (message, bytes) = message.into_inner();

            let sys_msg = SystemMessage::from_log_transfer_message(message);

            let serialized_msg = SerializedMessage::new(sys_msg, bytes);

            map.insert(node, StoredMessage::new(header, serialized_msg));
        }

        self.0.outgoing_stub().broadcast_serialized(map)
    }
}

impl<NI, D, P, L, VT, NT> ViewTransferProtocolSendNode<VT> for ProtocolNode<NI, D, P, L, VT, NT>
where
    D: ApplicationData,
    P: OrderingProtocolMessage<SMRReq<D>>,
    L: LogTransferMessage<SMRReq<D>, P>,
    VT: ViewTransferProtocolMessage,
    NT: RegularNetworkStub<Service<D, P, L, VT>>,
    NI: NetworkInformationProvider + 'static,
{
    type NetworkInfoProvider = NI;

    #[inline(always)]
    fn id(&self) -> NodeId {
        self.0.id()
    }

    #[inline(always)]
    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider> {
        &self.1
    }

    #[inline(always)]
    fn send(
        &self,
        message: VT::ProtocolMessage,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        self.0.outgoing_stub().send(
            SystemMessage::from_view_transfer_message(message),
            target,
            flush,
        )
    }

    #[inline(always)]
    fn send_signed(
        &self,
        message: VT::ProtocolMessage,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        self.0.outgoing_stub().send_signed(
            SystemMessage::from_view_transfer_message(message),
            target,
            flush,
        )
    }

    #[inline(always)]
    fn broadcast(
        &self,
        message: VT::ProtocolMessage,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        self.0
            .outgoing_stub()
            .broadcast(SystemMessage::from_view_transfer_message(message), targets)
    }

    #[inline(always)]
    fn broadcast_signed(
        &self,
        message: VT::ProtocolMessage,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        self.0
            .outgoing_stub()
            .broadcast_signed(SystemMessage::from_view_transfer_message(message), targets)
    }

    #[inline(always)]
    fn serialize_digest_message(
        &self,
        message: VT::ProtocolMessage,
    ) -> atlas_common::error::Result<(SerializedMessage<VT::ProtocolMessage>, Digest)> {
        let (message, digest) = self
            .0
            .outgoing_stub()
            .serialize_digest_message(SystemMessage::from_view_transfer_message(message))?;

        let (message, bytes): (ServiceMessage<D, P, L, VT>, Buf) = message.into_inner();

        let message = message.into_view_transfer_message();

        Ok((SerializedMessage::new(message, bytes), digest))
    }

    #[inline(always)]
    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<VT::ProtocolMessage>>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let mut map = BTreeMap::new();

        for (node, message) in messages.into_iter() {
            let (header, message) = message.into_inner();

            let (message, bytes) = message.into_inner();

            let sys_msg = SystemMessage::from_view_transfer_message(message);

            let serialized_msg = SerializedMessage::new(sys_msg, bytes);

            map.insert(node, StoredMessage::new(header, serialized_msg));
        }

        self.0.outgoing_stub().broadcast_serialized(map)
    }
}

impl<D, NT> RequestPreProcessingHandle<SMRSysMessage<D>> for AppNode<D, NT>
where
    D: ApplicationData + 'static,
    NT: BatchedNetworkStub<SMRSysMsg<D>>,
{
    #[inline(always)]
    fn receive_from_clients(
        &self,
        timeout: Option<Duration>,
    ) -> atlas_common::error::Result<Vec<StoredMessage<SMRSysMessage<D>>>> {
        match timeout {
            None => self.0.incoming_stub().receive_messages(),
            Some(timeout) => self
                .0
                .incoming_stub()
                .try_receive_messages(Some(timeout))?
                .ok_or(anyhow!("Timeout reached")),
        }
    }

    #[inline(always)]
    fn try_receive_from_clients(
        &self,
    ) -> atlas_common::error::Result<Option<Vec<StoredMessage<SMRSysMessage<D>>>>> {
        self.0.incoming_stub().try_receive_messages(None)
    }
}

impl<D, NT> ReplyNode<SMRReply<D>> for AppNode<D, NT>
where
    D: ApplicationData + 'static,
    NT: BatchedNetworkStub<SMRSysMsg<D>>,
{
    #[inline(always)]
    fn send(
        &self,
        reply_type: RequestType,
        reply: SMRReply<D>,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        let message = match reply_type {
            RequestType::Ordered => OrderableMessage::OrderedReply(reply),
            RequestType::Unordered => OrderableMessage::UnorderedReply(reply),
        };

        self.0.outgoing_stub().send(message, target, flush)
    }

    #[inline(always)]
    fn send_signed(
        &self,
        reply_type: RequestType,
        reply: SMRReply<D>,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        let message = match reply_type {
            RequestType::Ordered => OrderableMessage::OrderedReply(reply),
            RequestType::Unordered => OrderableMessage::UnorderedReply(reply),
        };

        self.0.outgoing_stub().send_signed(message, target, flush)
    }

    #[inline(always)]
    fn broadcast(
        &self,
        reply_type: RequestType,
        reply: SMRReply<D>,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let message = match reply_type {
            RequestType::Ordered => OrderableMessage::OrderedReply(reply),
            RequestType::Unordered => OrderableMessage::UnorderedReply(reply),
        };

        self.0.outgoing_stub().broadcast(message, targets)
    }

    #[inline(always)]
    fn broadcast_signed(
        &self,
        reply_type: RequestType,
        reply: SMRReply<D>,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        let message = match reply_type {
            RequestType::Ordered => OrderableMessage::OrderedReply(reply),
            RequestType::Unordered => OrderableMessage::UnorderedReply(reply),
        };

        self.0.outgoing_stub().broadcast_signed(message, targets)
    }
}

impl<S, NT> NetworkStub<StateSys<S>> for StateTransferNode<S, NT>
where
    S: StateTransferMessage,
    NT: RegularNetworkStub<StateSys<S>>,
{
    type Outgoing = NT::Outgoing;
    type Connections = NT::Connections;

    #[inline(always)]
    fn id(&self) -> NodeId {
        self.0.id()
    }

    #[inline(always)]
    fn outgoing_stub(&self) -> &Self::Outgoing {
        self.0.outgoing_stub()
    }

    #[inline(always)]
    fn connections(&self) -> &Arc<Self::Connections> {
        self.0.connections()
    }
}

impl<S, NT> RegularNetworkStub<StateSys<S>> for StateTransferNode<S, NT>
where
    S: StateTransferMessage,
    NT: RegularNetworkStub<StateSys<S>>,
{
    type Incoming = NT::Incoming;

    #[inline(always)]
    fn incoming_stub(&self) -> &Self::Incoming {
        self.0.incoming_stub()
    }
}

impl<S, NT> StateTransferSendNode<S> for StateTransferNode<S, NT>
where
    S: StateTransferMessage,
    NT: RegularNetworkStub<StateSys<S>>,
{
    #[inline(always)]
    fn id(&self) -> NodeId {
        self.0.id()
    }

    #[inline(always)]
    fn send(
        &self,
        message: S::StateTransferMessage,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        self.0.outgoing_stub().send(message, target, flush)
    }

    #[inline(always)]
    fn send_signed(
        &self,
        message: S::StateTransferMessage,
        target: NodeId,
        flush: bool,
    ) -> atlas_common::error::Result<()> {
        self.0.outgoing_stub().send_signed(message, target, flush)
    }

    #[inline(always)]
    fn broadcast(
        &self,
        message: S::StateTransferMessage,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        self.0.outgoing_stub().broadcast(message, targets)
    }

    #[inline(always)]
    fn broadcast_signed(
        &self,
        message: S::StateTransferMessage,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        self.0.outgoing_stub().broadcast_signed(message, targets)
    }

    #[inline(always)]
    fn serialize_digest_message(
        &self,
        message: S::StateTransferMessage,
    ) -> atlas_common::error::Result<(SerializedMessage<S::StateTransferMessage>, Digest)> {
        self.0.outgoing_stub().serialize_digest_message(message)
    }

    #[inline(always)]
    fn broadcast_serialized(
        &self,
        messages: BTreeMap<NodeId, StoredSerializedMessage<S::StateTransferMessage>>,
    ) -> std::result::Result<(), Vec<NodeId>> {
        self.0.outgoing_stub().broadcast_serialized(messages)
    }
}
