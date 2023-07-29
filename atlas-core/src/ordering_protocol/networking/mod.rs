use std::collections::BTreeMap;
use std::sync::Arc;
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_common::error::*;
use atlas_communication::FullNetworkNode;
use atlas_communication::message::{SerializedMessage, StoredMessage, StoredSerializedProtocolMessage};
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_communication::serialize::{Buf, Serializable};
use atlas_execution::serialize::ApplicationData;
use crate::messages::{ForwardedRequestsMessage, SystemMessage};
use crate::serialize::{LogTransferMessage, OrderingProtocolMessage, ServiceMsg, StateTransferMessage};
use crate::smr::networking::NodeWrap;

pub trait OrderProtocolSendNode<D, OPM>: Send + Sync where D: ApplicationData + 'static, OPM: OrderingProtocolMessage {
    type NetworkInfoProvider: NetworkInformationProvider + 'static;

    fn id(&self) -> NodeId;

    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider>;

    /// Forward requests to the given targets
    fn forward_requests(&self, fwd_requests: ForwardedRequestsMessage<D::Request>, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> where D: ApplicationData + 'static;

    /// Sends a message to a given target.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send(&self, message: OPM::ProtocolMessage, target: NodeId, flush: bool) -> Result<()>;

    /// Sends a signed message to a given target
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send_signed(&self, message: OPM::ProtocolMessage, target: NodeId, flush: bool) -> Result<()>;

    /// Broadcast a message to all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast(&self, message: OPM::ProtocolMessage, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>>;

    /// Broadcast a signed message for all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_signed(&self, message: OPM::ProtocolMessage, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>>;

    /// Serialize a message to a given target.
    /// Creates the serialized byte buffer along with the header, so we can send it later.
    fn serialize_digest_message(&self, message: OPM::ProtocolMessage) -> Result<(SerializedMessage<OPM::ProtocolMessage>, Digest)>;

    /// Broadcast the serialized messages provided.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_serialized(&self, messages: BTreeMap<NodeId, StoredSerializedProtocolMessage<OPM::ProtocolMessage>>) -> std::result::Result<(), Vec<NodeId>>;
}

impl<NT, D, P, S, L, RM, NI> OrderProtocolSendNode<D, P> for NodeWrap<NT, D, P, S, L, NI, RM>
    where D: ApplicationData + 'static,
          P: OrderingProtocolMessage + 'static,
          S: StateTransferMessage + 'static,
          L: LogTransferMessage + 'static,
          RM: Serializable + 'static,
          NI: NetworkInformationProvider + 'static,
          NT: FullNetworkNode<NI, RM, ServiceMsg<D, P, S, L>>, {

    type NetworkInfoProvider = <NT as ProtocolNetworkNode<ServiceMsg<D, P, S, L>>>::NetworkInfoProvider;

    #[inline(always)]
    fn id(&self) -> NodeId {
        self.0.id()
    }

    fn network_info_provider(&self) -> &Arc<Self::NetworkInfoProvider> {
        ProtocolNetworkNode::network_info_provider(&self.0)
    }

    fn forward_requests(&self, fwd_requests: ForwardedRequestsMessage<D::Request>, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> where D: ApplicationData + 'static {
        self.0.broadcast_signed(SystemMessage::ForwardedRequestMessage(fwd_requests), targets)
    }

    #[inline(always)]
    fn send(&self, message: P::ProtocolMessage, target: NodeId, flush: bool) -> Result<()> {
        self.0.send(SystemMessage::from_protocol_message(message), target, flush)
    }

    #[inline(always)]
    fn send_signed(&self, message: P::ProtocolMessage, target: NodeId, flush: bool) -> Result<()> {
        self.0.send_signed(SystemMessage::from_protocol_message(message), target, flush)
    }

    fn broadcast(&self, message: P::ProtocolMessage, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        self.0.broadcast(SystemMessage::from_protocol_message(message), targets)
    }

    fn broadcast_signed(&self, message: P::ProtocolMessage, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>> {
        self.0.broadcast_signed(SystemMessage::from_protocol_message(message), targets)
    }

    /// Why do we do this wrapping/unwrapping? Well, since we want to avoid having to store all of the
    /// generics that are used at the replica level (with all message types), we can't
    /// just return a system message type.
    /// This way, we can still keep this working well with just very small memory changes (to the stack)
    /// and avoid having to store all those unnecessary types in generics
    #[inline(always)]
    fn serialize_digest_message(&self, message: P::ProtocolMessage) -> Result<(SerializedMessage<P::ProtocolMessage>, Digest)> {
        let (message, digest) = self.0.serialize_digest_message(SystemMessage::from_protocol_message(message))?;

        let (message, bytes) = message.into_inner();

        let message = message.into_protocol_message();

        Ok((SerializedMessage::new(message, bytes), digest))
    }

    /// Read comment above
    #[inline(always)]
    fn broadcast_serialized(&self, messages: BTreeMap<NodeId, StoredSerializedProtocolMessage<P::ProtocolMessage>>) -> std::result::Result<(), Vec<NodeId>> {
        let mut map = BTreeMap::new();

        for (node, message) in messages.into_iter() {
            let (header, message) = message.into_inner();

            let (message, bytes) = message.into_inner();

            let sys_msg = SystemMessage::from_protocol_message(message);

            let serialized_msg = SerializedMessage::new(sys_msg, bytes);

            map.insert(node, StoredMessage::new(header, serialized_msg));
        }

        self.0.broadcast_serialized(map)
    }
}