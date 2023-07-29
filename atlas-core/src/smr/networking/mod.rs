use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_communication::FullNetworkNode;
use atlas_communication::message::{SerializedMessage, StoredSerializedProtocolMessage};
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_communication::reconfiguration_node::{NetworkInformationProvider, ReconfigurationNode};
use atlas_communication::serialize::Serializable;
use atlas_execution::serialize::ApplicationData;
use crate::log_transfer::networking::LogTransferSendNode;
use crate::ordering_protocol::networking::OrderProtocolSendNode;
use crate::serialize::{LogTransferMessage, OrderingProtocolMessage, ServiceMessage, ServiceMsg, StateTransferMessage};
use crate::smr::exec::ReplyNode;
use crate::state_transfer::networking::StateTransferSendNode;

///TODO: I wound up creating a whole new layer of abstractions, but I'm not sure they are necessary. I did it
/// To allow for the protocols to all use NT, as if I didn't, a lot of things would have to change in how the generic NT was
/// going to be passed around the protocols. I'm not sure if this is the best way to do it, but it works for now.
pub trait SMRNetworkNode<NI, RM, D, P, S, L>: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + ReplyNode<D> + StateTransferSendNode<S> + OrderProtocolSendNode<D, P> + LogTransferSendNode<L>
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static {}

#[derive(Clone)]
pub struct NodeWrap<NT, D, P, S, L, NI, RM>(pub NT, PhantomData<(D, P, S, L, NI, RM)>)
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + 'static,;

impl<NT, D, P, S, L, NI, RM> NodeWrap<NT, D, P, S, L, NI, RM>
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + 'static, {
    pub fn from_node(node: NT) -> Self {
        NodeWrap(node, Default::default())
    }
}

impl<NT, D, P, S, L, NI, RM> Deref for NodeWrap<NT, D, P, S, L, NI, RM>
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + 'static, {
    type Target = NT;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<NT, D, P, S, L, NI, RM> ProtocolNetworkNode<ServiceMsg<D, P, S, L>> for NodeWrap<NT, D, P, S, L, NI, RM>
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + 'static, {
    type ConnectionManager = <NT as ProtocolNetworkNode<ServiceMsg<D, P, S, L>>>::ConnectionManager;
    type NetworkInfoProvider = <NT as ProtocolNetworkNode<ServiceMsg<D, P, S, L>>>::NetworkInfoProvider;
    type IncomingRqHandler = NT::IncomingRqHandler;

    fn id(&self) -> NodeId {
        ProtocolNetworkNode::id(&self.0)
    }

    fn node_connections(&self) -> &Arc<Self::ConnectionManager> {
        ProtocolNetworkNode::node_connections(&self.0)
    }

    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider> {
        ProtocolNetworkNode::network_info_provider(&self.0)
    }

    fn node_incoming_rq_handling(&self) -> &Arc<Self::IncomingRqHandler> {
        ProtocolNetworkNode::node_incoming_rq_handling(&self.0)
    }

    fn send(&self, message: ServiceMessage<D, P, S, L>, target: NodeId, flush: bool) -> atlas_common::error::Result<()> {
        self.0.send(message, target, flush)
    }

    fn send_signed(&self, message: ServiceMessage<D, P, S, L>, target: NodeId, flush: bool) -> atlas_common::error::Result<()> {
        self.0.send_signed(message, target, flush)
    }

    fn broadcast(&self, message: ServiceMessage<D, P, S, L>, targets: impl Iterator<Item=NodeId>) -> Result<(), Vec<NodeId>> {
        self.0.broadcast(message, targets)
    }

    fn broadcast_signed(&self, message: ServiceMessage<D, P, S, L>, target: impl Iterator<Item=NodeId>) -> Result<(), Vec<NodeId>> {
        self.0.broadcast_signed(message, target)
    }

    fn serialize_digest_message(&self, message: ServiceMessage<D, P, S, L>) -> atlas_common::error::Result<(SerializedMessage<ServiceMessage<D, P, S, L>>, Digest)> {
        self.0.serialize_digest_message(message)
    }

    fn broadcast_serialized(&self, messages: BTreeMap<NodeId, StoredSerializedProtocolMessage<ServiceMessage<D, P, S, L>>>) -> Result<(), Vec<NodeId>> {
        self.0.broadcast_serialized(messages)
    }
}

impl<NT, D, P, S, L, NI, RM> ReconfigurationNode<RM> for NodeWrap<NT, D, P, S, L, NI, RM>
    where NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + 'static,
          D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          RM: Serializable + 'static, {
    type ConnectionManager = <NT as ReconfigurationNode<RM>>::ConnectionManager;
    type NetworkInfoProvider = <NT as ReconfigurationNode<RM>>::NetworkInfoProvider;
    type IncomingReconfigRqHandler = NT::IncomingReconfigRqHandler;
    type ReconfigurationNetworkUpdate = NT::ReconfigurationNetworkUpdate;

    fn node_connections(&self) -> &Arc<Self::ConnectionManager> {
        ReconfigurationNode::node_connections(&self.0)
    }

    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider> {
        ReconfigurationNode::network_info_provider(&self.0)
    }

    fn reconfiguration_network_update(&self) -> &Arc<Self::ReconfigurationNetworkUpdate> {
        self.0.reconfiguration_network_update()
    }

    fn reconfiguration_message_handler(&self) -> &Arc<Self::IncomingReconfigRqHandler> {
        self.0.reconfiguration_message_handler()
    }

    fn send_reconfig_message(&self, message: RM::Message, target: NodeId) -> atlas_common::error::Result<()> {
        self.0.send_reconfig_message(message, target)
    }

    fn broadcast_reconfig_message(&self, message: RM::Message, target: impl Iterator<Item=NodeId>) -> Result<(), Vec<NodeId>> {
        self.0.broadcast_reconfig_message(message, target)
    }
}

impl<NT, D, P, S, L, NI, RM> FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> for NodeWrap<NT, D, P, S, L, NI, RM>
    where
        D: ApplicationData + 'static,
        P: OrderingProtocolMessage + 'static,
        S: StateTransferMessage + 'static,
        L: LogTransferMessage + 'static,
        RM: Serializable + 'static,
        NI: NetworkInformationProvider + 'static,
        NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>>, {
    type Config = NT::Config;

    async fn bootstrap(network_info_provider: Arc<NI>, node_config: Self::Config) -> atlas_common::error::Result<Self> {
        Ok(NodeWrap::from_node(NT::bootstrap(network_info_provider, node_config).await?))
    }
}

impl<NT, NI, RM, D, P, S, L> SMRNetworkNode<NI, RM, D, P, S, L> for NodeWrap<NT, D, P, S, L, NI, RM>
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          NI: NetworkInformationProvider + 'static,
          RM: Serializable + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>> + 'static, {}