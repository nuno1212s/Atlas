#![allow(
    clippy::non_canonical_partial_ord_impl,
    clippy::large_enum_variant,
    type_alias_bounds
)]

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::messages::ClientRqInfo;
use crate::ordering_protocol::networking::serialize::{
    OrderingProtocolMessage, PermissionedOrderingProtocolMessage,
};
use crate::request_pre_processing::BatchOutput;
use crate::timeouts::timeout::{TimeoutModHandle, TimeoutableMod};
use anyhow::anyhow;
use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::maybe_vec::ordered::{MaybeOrderedVec, MaybeOrderedVecBuilder};
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::StoredMessage;
use atlas_metrics::benchmarks::BatchMeta;
use getset::Getters;

pub mod loggable;
pub mod networking;
pub mod permissioned;
pub mod reconfigurable_order_protocol;

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

/// Containment of a batch of messages
#[derive(Clone)]
pub struct BatchedDecision<RQ> {
    seq: SeqNo,
    inner: Vec<StoredMessage<RQ>>,
    meta: Option<BatchMeta>,
}

/// A given decision and information about it
/// To be taken by the replica and processed accordingly
pub struct Decision<MD, DAD, PM, RQ> {
    // The seq no of the decision
    seq: SeqNo,
    // Information about the decision progression.
    // Multiple instances can be bundled here in case we want to include various updates to
    // The same decision
    // This has to be in an ordered state as it is very important that the decision done
    // Enum always needs to be the last one delivered in order to even make sense
    decision_info: MaybeOrderedVec<DecisionInfo<MD, DAD, PM, RQ>>,
}

/// Partial information about the decision that is being progressed
#[derive(Getters)]
pub struct PartialDecisionInformation<DP, PM> {
    #[get = "pub"]
    message_partial_info: MaybeVec<DP>,
    #[get = "pub"]
    messages: MaybeVec<ShareableMessage<PM>>,
}

/// Information about a given decision
pub enum DecisionInfo<MD, DAD, PM, RQ> {
    // The decision metadata, does not indicate that the decision is made
    DecisionMetadata(MD),
    // Partial information about the decision (composing messages)
    PartialDecisionInformation(PartialDecisionInformation<DAD, PM>),
    // The decision has been completed
    DecisionDone(ProtocolConsensusDecision<RQ>),
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

/// The struct representing a consensus decision
///
/// Executable Batch: All of the requests that should be executed, in the correct order
/// Execution Result: Whether we need to ask the executor for a checkpoint in order to reset the current message log
/// Batch info: The information collected by the [DecidingLog], if applicable. (We can receive a batch
/// via a complete proof which means this will be [None] or we can process a batch normally, which means
/// this will be [Some(CompletedBatch<D>)])
pub struct ProtocolConsensusDecision<O> {
    seq: SeqNo,
    // The digest of the batch
    batch_digest: Digest,
    // The client requests information contained in the batch.
    contained_requests: Vec<ClientRqInfo>,
    // The batch of client requests to execute as a result of this protocol
    executable_batch: BatchedDecision<O>,
}

impl<MD, DAD, P, O> Decision<MD, DAD, P, O> {
    /// Create a decision information object from a stored message
    pub fn decision_info_from_message(seq: SeqNo, decision: ShareableMessage<P>) -> Self {
        Decision {
            seq,
            decision_info: MaybeOrderedVec::One(DecisionInfo::PartialDecisionInformation(
                decision.into(),
            )),
        }
    }

    /// Create a decision information object from a metadata object
    pub fn decision_info_from_metadata(seq: SeqNo, metadata: MD) -> Self {
        Decision {
            seq,
            decision_info: MaybeOrderedVec::One(DecisionInfo::DecisionMetadata(metadata)),
        }
    }

    /// Create a decision information object from a group of messages
    pub fn decision_info_from_messages(seq: SeqNo, messages: Vec<ShareableMessage<P>>) -> Self {
        Decision {
            seq,
            decision_info: MaybeOrderedVec::One(DecisionInfo::PartialDecisionInformation(
                messages.into(),
            )),
        }
    }

    /// Partial decision information creation
    pub fn partial_decision_info(
        seq: SeqNo,
        additional_data: MaybeVec<DAD>,
        messages: MaybeVec<ShareableMessage<P>>,
    ) -> Self {
        Decision {
            seq,
            decision_info: MaybeOrderedVec::One(DecisionInfo::PartialDecisionInformation(
                PartialDecisionInformation::new(additional_data, messages),
            )),
        }
    }

    /// Create a decision info from metadata and messages
    pub fn decision_info_from_metadata_and_messages(
        seq: SeqNo,
        metadata: MD,
        additional_data: MaybeVec<DAD>,
        messages: MaybeVec<ShareableMessage<P>>,
    ) -> Self
    where
        DAD: PartialEq,
    {
        let mut decision_info = BTreeSet::new();

        decision_info.insert(DecisionInfo::DecisionMetadata(metadata));
        decision_info.insert(DecisionInfo::PartialDecisionInformation(
            PartialDecisionInformation::new(additional_data, messages),
        ));

        Decision {
            seq,
            decision_info: MaybeOrderedVec::Mult(decision_info),
        }
    }

    /// Create a decision done object
    pub fn completed_decision(seq: SeqNo, update: ProtocolConsensusDecision<O>) -> Self {
        Decision {
            seq,
            decision_info: MaybeOrderedVec::One(DecisionInfo::DecisionDone(update)),
        }
    }

    /// Create a full decision info, from all of the components
    pub fn full_decision_info(
        seq: SeqNo,
        metadata: MD,
        additional_metric_data: MaybeVec<DAD>,
        messages: MaybeVec<ShareableMessage<P>>,
        requests: ProtocolConsensusDecision<O>,
    ) -> Self
    where
        DAD: PartialEq,
    {
        let mut decision_info = BTreeSet::new();

        decision_info.insert(DecisionInfo::DecisionMetadata(metadata));
        decision_info.insert(DecisionInfo::PartialDecisionInformation(
            PartialDecisionInformation::new(additional_metric_data, messages),
        ));
        decision_info.insert(DecisionInfo::DecisionDone(requests));

        Decision {
            seq,
            decision_info: MaybeOrderedVec::from_set(decision_info),
        }
    }

    /// Merge two decisions by appending one to the other
    /// Returns an error when the sequence number of the decisions does not match
    pub fn merge_decisions(&mut self, other: Self) -> Result<()>
    where
        DAD: PartialEq,
    {
        if self.seq != other.seq {
            return Err(anyhow!(
                "The decisions have different sequence numbers, cannot merge"
            ));
        }

        let mut ordered_vec_builder = MaybeOrderedVecBuilder::from_existing(other.decision_info);

        self.decision_info = {
            let decisions = std::mem::replace(&mut self.decision_info, MaybeOrderedVec::None);

            for dec_info in decisions.into_iter() {
                ordered_vec_builder.push(dec_info);
            }

            ordered_vec_builder.build()
        };

        Ok(())
    }

    pub fn append_decision_info(&mut self, decision_info: DecisionInfo<MD, DAD, P, O>)
    where
        DAD: PartialEq,
    {
        self.decision_info = {
            let decisions = std::mem::replace(&mut self.decision_info, MaybeOrderedVec::None);

            let mut decisions = MaybeOrderedVecBuilder::from_existing(decisions);

            decisions.push(decision_info);

            decisions.build()
        };
    }

    pub fn decision_info(&self) -> &MaybeOrderedVec<DecisionInfo<MD, DAD, P, O>> {
        &self.decision_info
    }

    pub fn into_decision_info(self) -> MaybeOrderedVec<DecisionInfo<MD, DAD, P, O>> {
        self.decision_info
    }
}

impl<RQ> Orderable for BatchedDecision<RQ> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl<MD, DAD, P, O> Orderable for Decision<MD, DAD, P, O> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl<MD, DAD, P, O> Debug for Decision<MD, DAD, P, O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Decision {:?}. Infos {:?}", self.seq, self.decision_info)
    }
}

impl<MD, DAD, P, O> DecisionInfo<MD, DAD, P, O> {
    pub fn decision_info_from_message(message: MaybeVec<ShareableMessage<P>>) -> MaybeVec<Self> {
        MaybeVec::from_one(Self::PartialDecisionInformation(
            PartialDecisionInformation::new(MaybeVec::None, message),
        ))
    }

    pub fn decision_info_from_message_and_metadata(
        message: MaybeVec<ShareableMessage<P>>,
        metadata: MD,
    ) -> MaybeVec<Self> {
        let partial = Self::PartialDecisionInformation(PartialDecisionInformation::new(
            MaybeVec::None,
            message,
        ));
        let metadata = Self::DecisionMetadata(metadata);

        MaybeVec::Mult(vec![partial, metadata])
    }
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

impl<MD, DAD, P, O> PartialEq<Self> for DecisionInfo<MD, DAD, P, O>
where
    DAD: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DecisionInfo::DecisionMetadata(_md), DecisionInfo::DecisionMetadata(_md2)) => true,
            (DecisionInfo::DecisionDone(_prot), DecisionInfo::DecisionDone(_prot2)) => true,
            (
                DecisionInfo::PartialDecisionInformation(info),
                DecisionInfo::PartialDecisionInformation(info2),
            ) => {
                if info.messages().len() != info2.messages().len()
                    || info.message_partial_info().len() != info2.message_partial_info().len()
                {
                    false
                } else {
                    let is_partial_info_eq = info.message_partial_info().iter().all(|p_info| {
                        info2
                            .message_partial_info()
                            .iter()
                            .any(|p_info_2| *p_info_2 == *p_info)
                    });

                    let is_messages_eq = info.messages().iter().all(|msg| {
                        info2
                            .messages()
                            .iter()
                            .any(|msg_2| msg.header().digest() == msg_2.header().digest())
                    });

                    if !is_partial_info_eq || !is_messages_eq {
                        return false;
                    }

                    true
                }
            }
            (_, _) => false,
        }
    }
}

impl<MD, DAD, P, O> PartialOrd for DecisionInfo<MD, DAD, P, O>
where
    DAD: PartialEq,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (DecisionInfo::DecisionDone(_), _) => Some(Ordering::Greater),
            (DecisionInfo::DecisionMetadata(_), DecisionInfo::DecisionDone(_)) => {
                Some(Ordering::Less)
            }
            (DecisionInfo::DecisionMetadata(_), DecisionInfo::DecisionMetadata(_)) => {
                Some(Ordering::Equal)
            }
            (DecisionInfo::DecisionMetadata(_), _) => Some(Ordering::Greater),
            (DecisionInfo::PartialDecisionInformation(_), _) => Some(Ordering::Less),
        }
    }
}

impl<MD, DAD, P, O> Eq for DecisionInfo<MD, DAD, P, O> where DAD: PartialEq {}

impl<MD, DAD, P, O> Ord for DecisionInfo<MD, DAD, P, O>
where
    DAD: PartialEq,
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}

/// Constructor for the ProtocolConsensusDecision struct
impl<O> ProtocolConsensusDecision<O> {
    pub fn new(
        seq: SeqNo,
        executable_batch: BatchedDecision<O>,
        client_rqs: Vec<ClientRqInfo>,
        batch_digest: Digest,
    ) -> Self {
        ProtocolConsensusDecision {
            seq,
            batch_digest,
            contained_requests: client_rqs,
            executable_batch,
        }
    }

    pub fn into(self) -> (SeqNo, BatchedDecision<O>, Vec<ClientRqInfo>, Digest) {
        (
            self.seq,
            self.executable_batch,
            self.contained_requests,
            self.batch_digest,
        )
    }

    pub fn update_batch(&self) -> &BatchedDecision<O> {
        &self.executable_batch
    }
}

/// Unwrap a shareable message, avoiding cloning at all costs
pub fn unwrap_shareable_message<T: Clone>(message: ShareableMessage<T>) -> StoredMessage<T> {
    Arc::try_unwrap(message).unwrap_or_else(|pointer| (*pointer).clone())
}

impl<O> Orderable for ProtocolConsensusDecision<O> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl<O> Debug for ProtocolConsensusDecision<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ProtocolConsensusDecision {{ seq: {:?}, executable_batch: {:?}, batch_info: {:?} }}",
            self.seq,
            self.executable_batch.len(),
            self.batch_digest
        )
    }
}

impl<MD, DAD, D, P> Debug for DecisionInfo<MD, DAD, D, P> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionInfo::DecisionMetadata(_) => {
                write!(f, "Decision metadata ")
            }
            DecisionInfo::PartialDecisionInformation(_) => {
                write!(f, "Partial dec")
            }
            DecisionInfo::DecisionDone(_) => {
                write!(f, "Decision Done")
            }
        }
    }
}

impl<RQ> BatchedDecision<RQ> {
    pub fn new(seq: SeqNo, batch: Vec<StoredMessage<RQ>>, meta: Option<BatchMeta>) -> Self {
        BatchedDecision {
            seq,
            inner: batch,
            meta,
        }
    }

    pub fn new_with_cap(seq: SeqNo, capacity: usize) -> Self {
        BatchedDecision {
            seq,
            inner: Vec::with_capacity(capacity),
            meta: None,
        }
    }

    pub fn new_with_batch(seq: SeqNo, batch: Vec<StoredMessage<RQ>>) -> Self {
        BatchedDecision {
            seq,
            inner: batch,
            meta: None,
        }
    }

    pub fn into_inner(self) -> Vec<StoredMessage<RQ>> {
        self.inner
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn add_message(&mut self, message: StoredMessage<RQ>) {
        self.inner.push(message);
    }

    pub fn meta(&self) -> Option<&BatchMeta> {
        self.meta.as_ref()
    }

    pub fn append_batch_meta(&mut self, batch_meta: BatchMeta) {
        let _ = self.meta.insert(batch_meta);
    }

    pub fn take_meta(&mut self) -> Option<BatchMeta> {
        self.meta.take()
    }
}

impl<DAD, PM> PartialDecisionInformation<DAD, PM> {
    pub fn new(
        message_partial_info: MaybeVec<DAD>,
        messages: MaybeVec<ShareableMessage<PM>>,
    ) -> Self {
        Self {
            message_partial_info,
            messages,
        }
    }
}

impl<DAD, PM> From<PartialDecisionInformation<DAD, PM>>
    for (MaybeVec<DAD>, MaybeVec<Arc<StoredMessage<PM>>>)
{
    fn from(value: PartialDecisionInformation<DAD, PM>) -> Self {
        (value.message_partial_info, value.messages)
    }
}

impl<DAD, PM> From<Vec<ShareableMessage<PM>>> for PartialDecisionInformation<DAD, PM> {
    fn from(value: Vec<ShareableMessage<PM>>) -> Self {
        Self {
            message_partial_info: MaybeVec::None,
            messages: MaybeVec::from_many(value),
        }
    }
}

impl<DAD, PM> From<ShareableMessage<PM>> for PartialDecisionInformation<DAD, PM> {
    fn from(value: ShareableMessage<PM>) -> Self {
        Self {
            message_partial_info: MaybeVec::None,
            messages: MaybeVec::from_one(value),
        }
    }
}

/*
impl<RQ> From<Vec<StoredMessage<RQ>>> for BatchedDecision<RQ> {
    fn from(value: Vec<StoredMessage<RQ>>) -> Self {
        BatchedDecision {
            seq: (),
            inner: value,
            meta: None,
        }
    }
}*/
