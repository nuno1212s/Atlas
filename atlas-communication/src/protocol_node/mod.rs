use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use atlas_common::node_id::NodeId;
use atlas_common::error::*;
use crate::{NodeConnections, NodePK};
use crate::message::{StoredMessage, StoredSerializedProtocolMessage};
use crate::serialize::Serializable;


/// Trait for taking requests from the network node
/// We separate the various sources of requests in order to
/// allow for better handling of the requests
pub trait NodeIncomingRqHandler<T>: Send {
    fn rqs_len_from_clients(&self) -> usize;

    fn receive_from_clients(&self, timeout: Option<Duration>) -> Result<Vec<T>>;

    fn try_receive_from_clients(&self) -> Result<Option<Vec<T>>>;

    fn rqs_len_from_replicas(&self) -> usize;

    fn receive_from_replicas(&self, timeout: Option<Duration>) -> Result<Option<T>>;
}

/// A Network node devoted to handling
pub trait ProtocolNetworkNode<M>: Send + Sync where M: Serializable + 'static{

    type ConnectionManager: NodeConnections;

    type Crypto: NodePK;

    type IncomingRqHandler: NodeIncomingRqHandler<StoredMessage<M::Message>>;

    /// Reports the id of this `Node`.
    fn id(&self) -> NodeId;

    /// Reports the first Id
    fn first_cli(&self) -> NodeId;

    /// Get a handle to the connection manager of this node.
    fn node_connections(&self) -> &Arc<Self::ConnectionManager>;

    /// Crypto
    fn pk_crypto(&self) -> &Arc<Self::Crypto>;

    /// Get a reference to the incoming request handling
    fn node_incoming_rq_handling(&self) -> &Arc<Self::IncomingRqHandler>;

    /// Sends a message to a given target.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send(&self, message: M::Message, target: NodeId, flush: bool) -> Result<()>;

    /// Sends a signed message to a given target
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the target or err if not. No other checks are made
    /// on the success of the message dispatch
    fn send_signed(&self, message: M::Message, target: NodeId, flush: bool) -> Result<()>;

    /// Broadcast a message to all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast(&self, message: M::Message, targets: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>>;

    /// Broadcast a signed message for all of the given targets
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_signed(&self, message: M::Message, target: impl Iterator<Item=NodeId>) -> std::result::Result<(), Vec<NodeId>>;

    /// Serialize a message to a given target.
    /// Creates the serialized byte buffer along with the header, so we can send it later.
    fn serialize_sign_message(&self, message: M::Message, target: NodeId) -> Result<StoredSerializedProtocolMessage<M>>;

    /// Broadcast the serialized messages provided.
    /// Does not block on the message sent. Returns a result that is
    /// Ok if there is a current connection to the targets or err if not. No other checks are made
    /// on the success of the message dispatch
    fn broadcast_serialized(&self, messages: BTreeMap<NodeId, StoredSerializedProtocolMessage<M::Message>>) -> std::result::Result<(), Vec<NodeId>>;
}
