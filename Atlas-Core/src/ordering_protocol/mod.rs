#![allow(
    clippy::non_canonical_partial_ord_impl,
    clippy::large_enum_variant,
    type_alias_bounds
)]

use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::ordering_protocol::networking::serialize::{
    OrderingProtocolMessage, PermissionedOrderingProtocolMessage,
};
use crate::request_pre_processing::BatchOutput;
use crate::timeouts::timeout::{TimeoutModHandle, TimeoutableMod};
use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::StoredMessage;
use decision::Decision;

pub mod loggable;
pub mod networking;
pub mod permissioned;
pub mod reconfigurable_order_protocol;
pub mod decision;

pub type View<POP: PermissionedOrderingProtocolMessage> =
    <POP as PermissionedOrderingProtocolMessage>::ViewInfo;
pub type ShareableConsensusMessage<RQ, OP> =
    Arc<StoredMessage<<OP as OrderingProtocolMessage<RQ>>::ProtocolMessage>>;
pub type ShareableMessage<P> = Arc<StoredMessage<P>>;

pub type ProtocolMessage<RQ, OP> = <OP as OrderingProtocolMessage<RQ>>::ProtocolMessage;
pub type DecisionMetadata<RQ, OP> = <OP as OrderingProtocolMessage<RQ>>::DecisionMetadata;
pub type DecisionAD<RQ, OP> = <OP as OrderingProtocolMessage<RQ>>::DecisionAdditionalInfo;

/// The arguments that are necessary for the ordering protocol to be initialized
/// The R request type,
/// The RQPP: Request pre processor
/// The NT: Network
pub struct OrderingProtocolArgs<R, RQPP, NT>(
    pub NodeId,
    pub TimeoutModHandle,
    pub RQPP,
    pub BatchOutput<R>,
    pub Arc<NT>,
    pub Vec<NodeId>,
);

/// A trait that specifies how many nodes are necessary
/// in order to tolerate f failures
pub trait OrderProtocolTolerance {
    /// Get the amount of nodes necessary to tolerate f faults
    fn get_n_for_f(f: usize) -> usize;

    /// Get the quorum of nodes that N nodes with this protocol
    /// can tolerate
    fn get_quorum_for_n(n: usize) -> usize;

    fn get_f_for_n(n: usize) -> usize;
}

pub type OPResult<RQ, SER> =
    OPPollResult<DecisionMetadata<RQ, SER>, DecisionAD<RQ, SER>, ProtocolMessage<RQ, SER>, RQ>;
pub type OPExResult<RQ, SER> =
    OPExecResult<DecisionMetadata<RQ, SER>, DecisionAD<RQ, SER>, ProtocolMessage<RQ, SER>, RQ>;

/// The trait for an ordering protocol to be implemented in Atlas
///
/// An ordering protocol is meant to order various requests (of type RQ) received
/// into a globally accepted order in a fault tolerant scenario, which is can then be used by FT applications
///
/// The generic type presented here is the type of the request that the ordering protocol will be ordering
/// This can be whatever the developer wants, as long as it implements the [SerMsg] trait
pub trait OrderingProtocol<RQ>:
    OrderProtocolTolerance + Orderable + TimeoutableMod<OPExResult<RQ, Self::Serialization>>
{
    /// The type which implements OrderingProtocolMessage, to be implemented by the developer
    type Serialization: OrderingProtocolMessage<RQ> + 'static;

    /// The configuration type the protocol wants to accept
    type Config;

    /// Handle a protocol message that was received while we are executing another protocol
    fn handle_off_ctx_message(
        &mut self,
        message: ShareableConsensusMessage<RQ, Self::Serialization>,
    );

    /// Handle the protocol being executed having changed (for example to the state transfer protocol)
    /// This is important for some of the protocols, which need to know when they are being executed or not
    fn handle_execution_changed(&mut self, is_executing: bool) -> Result<()>;

    /// Poll from the ordering protocol in order to know what we should do next
    /// We do this to check if there are already messages waiting to be executed that were received ahead of time and stored.
    /// Or whether we should run state transfer or wait for messages from other replicas
    fn poll(&mut self) -> Result<OPResult<RQ, Self::Serialization>>;

    /// Process a protocol message that we have received
    /// This can be a message received from the poll() method or a message received from other replicas.
    fn process_message(
        &mut self,
        message: ShareableConsensusMessage<RQ, Self::Serialization>,
    ) -> Result<OPExResult<RQ, Self::Serialization>>;

    /// Install a given sequence number
    fn install_seq_no(&mut self, seq_no: SeqNo) -> Result<()>;
}

/// A permissioned ordering protocol, meaning only a select few are actually part of the quorum that decides the
/// ordering of the operations.
pub trait PermissionedOrderingProtocol: OrderProtocolTolerance {
    type PermissionedSerialization: PermissionedOrderingProtocolMessage + 'static;

    /// Get the current view of the ordering protocol
    fn view(&self) -> View<Self::PermissionedSerialization>;

    /// Install a given view into the ordering protocol
    fn install_view(&mut self, view: View<Self::PermissionedSerialization>);
}

#[derive(Debug, Clone)]
/// The information about a node having joined the quorum
pub struct JoinInfo {
    node: NodeId,
    new_quorum: Vec<NodeId>,
}

/// The return enum of polling the ordering protocol
pub enum OPPollResult<MD, DAD, P, O> {
    /// The order protocol requires the protocol to update its state to be inline with
    /// the rest of replicas in the system
    RunCst,
    /// The ordering protocol requires reception of messages from other nodes in order
    /// to progress
    ReceiveMsg,
    /// The order protocol had a message stored that was received out of order
    /// but is now ready to be processed
    Exec(ShareableMessage<P>),
    /// The ordered protocol progressed a decision as a result of the poll operation
    ProgressedDecision(DecisionsAhead, MaybeVec<Decision<MD, DAD, P, O>>),
    /// The quorum was joined as a result of the poll operation
    QuorumJoined(
        DecisionsAhead,
        Option<MaybeVec<Decision<MD, DAD, P, O>>>,
        JoinInfo,
    ),
    /// The order protocol wants to be polled again, for some particular reason
    RePoll,
}

/// What should be the action on the decisions that are not yet decided
///
#[derive(Debug)]
pub enum DecisionsAhead {
    Ignore,
    ClearAhead,
}

#[derive(Debug)]
/// The result of the ordering protocol executing a message
pub enum OPExecResult<MD, DAD, P, O> {
    /// The message we have passed onto the order protocol was dropped
    MessageDropped,
    /// The message we have passed onto the order protocol was queued for later use
    MessageQueued,
    /// The input was processed but there are no new updates to take from it
    MessageProcessedNoUpdate,
    /// The given decisions have been progressed (containing the progress information)
    ProgressedDecision(DecisionsAhead, MaybeVec<Decision<MD, DAD, P, O>>),
    /// The quorum has been joined by a given node.
    /// Do we want this to also clear the upcoming decisions?
    QuorumJoined(
        DecisionsAhead,
        Option<MaybeVec<Decision<MD, DAD, P, O>>>,
        JoinInfo,
    ),
    /// The order protocol requires the protocol to update its state to be inline with
    /// the rest of replicas in the system
    RunCst,
    //TODO: clear the deciding log from a seq number onwards in order to handle
    // view changes or such protocols
}

/// Information reported after a logging operation.
pub enum ExecutionResult {
    /// Nothing to report.
    Nil,
    /// The log became full. We are waiting for the execution layer
    /// to provide the current serialized application state, so we can
    /// complete the log's garbage collection and eventually its
    /// checkpoint.
    BeginCheckpoint,
}

impl JoinInfo {
    pub fn new(joined: NodeId, current_quorum: Vec<NodeId>) -> Self {
        Self {
            node: joined,
            new_quorum: current_quorum,
        }
    }

    pub fn into_inner(self) -> (NodeId, Vec<NodeId>) {
        (self.node, self.new_quorum)
    }
}

impl<MD, DAD, P, D> Debug for OPPollResult<MD, DAD, P, D>
where
    P: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            OPPollResult::RunCst => {
                write!(f, "RunCst")
            }
            OPPollResult::ReceiveMsg => {
                write!(f, "Receive From Replicas")
            }
            OPPollResult::Exec(message) => {
                write!(f, "Exec message {:?}", message.message())
            }
            OPPollResult::RePoll => {
                write!(f, "RePoll")
            }
            OPPollResult::ProgressedDecision(_clear_ahead, rqs) => {
                write!(f, "{} committed decisions", rqs.len())
            }
            OPPollResult::QuorumJoined(clear_ahead, decisions, node) => {
                let len = if let Some(vec) = decisions {
                    vec.len()
                } else {
                    0
                };

                write!(
                    f,
                    "Join information: {node:?}. Contained Decisions {len}, Clear Ahead {clear_ahead:?}"
                )
            }
        }
    }
}

/// Unwrap a shareable message, avoiding cloning at all costs
pub fn unwrap_shareable_message<T: Clone>(message: ShareableMessage<T>) -> StoredMessage<T> {
    Arc::try_unwrap(message).unwrap_or_else(|pointer| (*pointer).clone())
}