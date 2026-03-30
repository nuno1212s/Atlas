use crate::messages::ClientRqInfo;
use crate::ordering_protocol::ShareableMessage;
use anyhow::anyhow;
use atlas_common::crypto::hash::Digest;
use atlas_common::error;
use atlas_common::maybe_vec::ordered::{MaybeOrderedVec, MaybeOrderedVecBuilder};
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::StoredMessage;
use atlas_metrics::benchmarks::BatchMeta;
use getset::Getters;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

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
    decision_parts: MaybeOrderedVec<DecisionPart<MD, DAD, PM, RQ>>,
}

impl<MD, DAD, P, O> Decision<MD, DAD, P, O>
where
    DAD: PartialEq,
{
    /// Create a decision information object from a stored message
    #[must_use]
    pub fn decision_from_message(seq: SeqNo, decision: ShareableMessage<P>) -> Self {
        Decision {
            seq,
            decision_parts: MaybeOrderedVec::One(DecisionPart::PartialDecisionInformation(
                decision.into(),
            )),
        }
    }

    /// Create a decision information object from a metadata object
    #[must_use]
    pub fn decision_from_metadata(seq: SeqNo, metadata: MD) -> Self {
        Decision {
            seq,
            decision_parts: MaybeOrderedVec::One(DecisionPart::DecisionMetadata(metadata)),
        }
    }

    /// Create a decision information object from a group of messages
    #[must_use]
    pub fn decision_from_messages(seq: SeqNo, messages: Vec<ShareableMessage<P>>) -> Self {
        Decision {
            seq,
            decision_parts: MaybeOrderedVec::One(DecisionPart::PartialDecisionInformation(
                messages.into(),
            )),
        }
    }

    #[must_use]
    pub fn decision_from_requests(seq: SeqNo, requests: DecisionRequests<O>) -> Self {
        Decision {
            seq,
            decision_parts: MaybeOrderedVec::One(DecisionPart::DecisionRequests(requests)),
        }
    }

    #[must_use]
    pub fn decision_done(seq: SeqNo) -> Self {
        Decision {
            seq,
            decision_parts: MaybeOrderedVec::One(DecisionPart::DecisionDone),
        }
    }

    /// Partial decision information creation
    #[must_use]
    pub fn partial_decision_info(
        seq: SeqNo,
        additional_data: MaybeVec<DAD>,
        messages: MaybeVec<ShareableMessage<P>>,
    ) -> Self {
        Decision {
            seq,
            decision_parts: MaybeOrderedVec::One(DecisionPart::PartialDecisionInformation(
                PartialDecisionInformation::new(additional_data, messages),
            )),
        }
    }

    /// Create a decision info from metadata and messages
    #[must_use]
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

        decision_info.insert(DecisionPart::DecisionMetadata(metadata));
        decision_info.insert(DecisionPart::PartialDecisionInformation(
            PartialDecisionInformation::new(additional_data, messages),
        ));

        Decision {
            seq,
            decision_parts: MaybeOrderedVec::Mult(decision_info),
        }
    }

    #[must_use]
    pub fn decision_from_metadata_and_requests(
        seq: SeqNo,
        metadata: MD,
        additional_data: MaybeVec<DAD>,
        messages: MaybeVec<ShareableMessage<P>>,
        requests: DecisionRequests<O>,
    ) -> Self
    where
        DAD: PartialEq,
    {
        let mut decision_info = BTreeSet::new();

        decision_info.insert(DecisionPart::DecisionMetadata(metadata));
        decision_info.insert(DecisionPart::DecisionRequests(requests));
        decision_info.insert(DecisionPart::PartialDecisionInformation(
            PartialDecisionInformation::new(additional_data, messages),
        ));

        Decision {
            seq,
            decision_parts: MaybeOrderedVec::from_set(decision_info),
        }
    }

    /// Create a decision done object
    #[must_use]
    pub fn completed_decision_with_requests(seq: SeqNo, update: DecisionRequests<O>) -> Self {
        Decision {
            seq,
            decision_parts: MaybeOrderedVec::from_many(vec![
                DecisionPart::DecisionRequests(update),
                DecisionPart::DecisionDone,
            ]),
        }
    }

    /// Create a full decision info, from all of the components
    #[must_use]
    pub fn full_decision_info(
        seq: SeqNo,
        metadata: MD,
        additional_metric_data: MaybeVec<DAD>,
        messages: MaybeVec<ShareableMessage<P>>,
        requests: DecisionRequests<O>,
    ) -> Self
    where
        DAD: PartialEq,
    {
        let mut decision_info = BTreeSet::new();

        decision_info.insert(DecisionPart::DecisionMetadata(metadata));
        decision_info.insert(DecisionPart::PartialDecisionInformation(
            PartialDecisionInformation::new(additional_metric_data, messages),
        ));
        decision_info.insert(DecisionPart::DecisionRequests(requests));
        decision_info.insert(DecisionPart::DecisionDone);

        Decision {
            seq,
            decision_parts: MaybeOrderedVec::from_set(decision_info),
        }
    }

    /// Merge two decisions by appending one to the other
    /// Returns an error when the sequence number of the decisions does not match
    ///
    /// # Errors
    /// Returns an error if the sequence numbers of the decisions do not match
    #[must_use]
    pub fn merge_decisions(&mut self, other: Self) -> error::Result<()>
    where
        DAD: PartialEq,
    {
        if self.seq != other.seq {
            return Err(anyhow!(
                "The decisions have different sequence numbers, cannot merge"
            ));
        }

        let mut ordered_vec_builder = MaybeOrderedVecBuilder::from_existing(other.decision_parts);

        self.decision_parts = {
            let decisions = std::mem::replace(&mut self.decision_parts, MaybeOrderedVec::None);

            for dec_info in decisions {
                ordered_vec_builder.push(dec_info);
            }

            ordered_vec_builder.build()
        };

        Ok(())
    }

    pub fn append_decision_info(&mut self, decision_info: DecisionPart<MD, DAD, P, O>)
    where
        DAD: PartialEq,
    {
        self.decision_parts = {
            let decisions = std::mem::replace(&mut self.decision_parts, MaybeOrderedVec::None);

            let mut decisions = MaybeOrderedVecBuilder::from_existing(decisions);

            decisions.push(decision_info);

            decisions.build()
        };
    }

    pub fn decision_info(&self) -> &MaybeOrderedVec<DecisionPart<MD, DAD, P, O>> {
        &self.decision_parts
    }

    pub fn into_decision_info(self) -> MaybeOrderedVec<DecisionPart<MD, DAD, P, O>> {
        self.decision_parts
    }
}

impl<MD, DAD, P, O> Orderable for Decision<MD, DAD, P, O> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl<MD, DAD, P, O> Debug for Decision<MD, DAD, P, O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Decision {:?}. Infos {:?}",
            self.seq, self.decision_parts
        )
    }
}

/// Partial information about the decision that is being progressed
#[derive(Getters)]
pub struct PartialDecisionInformation<DAD, PM> {
    #[get = "pub"]
    message_partial_info: MaybeVec<DAD>,
    #[get = "pub"]
    messages: MaybeVec<ShareableMessage<PM>>,
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

/// Information about a given decision
pub enum DecisionPart<MD, DAD, PM, RQ> {
    // The decision metadata, does not indicate that the decision is made
    DecisionMetadata(MD),
    // Partial information about the decision (composing messages)
    PartialDecisionInformation(PartialDecisionInformation<DAD, PM>),
    // Requests contained within the decision of the protocol
    DecisionRequests(DecisionRequests<RQ>),
    // The decision has been completed
    DecisionDone,
}

impl<MD, DAD, P, O> DecisionPart<MD, DAD, P, O> {
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

    fn rank(d: &DecisionPart<MD, DAD, P, O>) -> u8 {
        match d {
            DecisionPart::DecisionMetadata(_) => 0,
            DecisionPart::PartialDecisionInformation(_) => 1,
            DecisionPart::DecisionRequests(_) => 2,
            DecisionPart::DecisionDone => 3,
        }
    }
}

impl<MD, DAD, P, O> PartialEq<Self> for DecisionPart<MD, DAD, P, O>
where
    DAD: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DecisionPart::DecisionMetadata(_md), DecisionPart::DecisionMetadata(_md2)) => true,
            (DecisionPart::DecisionDone, DecisionPart::DecisionDone) => true,
            (
                DecisionPart::PartialDecisionInformation(info),
                DecisionPart::PartialDecisionInformation(info2),
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

impl<MD, DAD, P, O> PartialOrd for DecisionPart<MD, DAD, P, O>
where
    DAD: PartialEq,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Self::rank(self).cmp(&Self::rank(other)))
    }
}

impl<MD, DAD, P, O> Eq for DecisionPart<MD, DAD, P, O> where DAD: PartialEq {}

impl<MD, DAD, P, O> Ord for DecisionPart<MD, DAD, P, O>
where
    DAD: PartialEq,
{
    fn cmp(&self, other: &Self) -> Ordering {
        Self::rank(self).cmp(&Self::rank(other))
    }
}

impl<MD, DAD, D, P> Debug for DecisionPart<MD, DAD, D, P> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionPart::DecisionMetadata(_) => {
                write!(f, "Decision metadata ")
            }
            DecisionPart::PartialDecisionInformation(_) => {
                write!(f, "Partial decision")
            }
            DecisionPart::DecisionDone => {
                write!(f, "Decision Done")
            }
            &DecisionPart::DecisionRequests(_) => write!(f, "Decision Requests"),
        }
    }
}

/// Struct which stores information about the client requests
/// belonging to a given decision the order protocol is currently
/// deciding about
#[derive(Clone)]
pub struct DecisionRequests<O> {
    seq: SeqNo,
    // The digest of the batch
    batch_digest: Digest,
    // The client requests information contained in the batch.
    contained_requests: Vec<ClientRqInfo>,
    // The batch of client requests to execute as a result of this protocol
    executable_batch: DecisionRequestBatch<O>,
}

impl<O> Orderable for DecisionRequests<O> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

/// Constructor for the ProtocolConsensusDecision struct
impl<O> DecisionRequests<O> {
    #[must_use]
    pub fn new(
        seq: SeqNo,
        executable_batch: DecisionRequestBatch<O>,
        client_rqs: Vec<ClientRqInfo>,
        batch_digest: Digest,
    ) -> Self {
        DecisionRequests {
            seq,
            batch_digest,
            contained_requests: client_rqs,
            executable_batch,
        }
    }

    #[must_use]
    pub fn into(self) -> (SeqNo, DecisionRequestBatch<O>, Vec<ClientRqInfo>, Digest) {
        (
            self.seq,
            self.executable_batch,
            self.contained_requests,
            self.batch_digest,
        )
    }

    pub fn update_batch(&self) -> &DecisionRequestBatch<O> {
        &self.executable_batch
    }
}

impl<O> Debug for DecisionRequests<O> {
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

/// Containment of a batch of client request messages
#[derive(Clone)]
pub struct DecisionRequestBatch<RQ> {
    seq: SeqNo,
    inner: Vec<StoredMessage<RQ>>,
    meta: Option<BatchMeta>,
}

impl<RQ> Orderable for DecisionRequestBatch<RQ> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl<RQ> DecisionRequestBatch<RQ> {
    pub fn new(seq: SeqNo, batch: Vec<StoredMessage<RQ>>, meta: Option<BatchMeta>) -> Self {
        DecisionRequestBatch {
            seq,
            inner: batch,
            meta,
        }
    }

    pub fn new_with_cap(seq: SeqNo, capacity: usize) -> Self {
        DecisionRequestBatch {
            seq,
            inner: Vec::with_capacity(capacity),
            meta: None,
        }
    }

    pub fn new_with_batch(seq: SeqNo, batch: Vec<StoredMessage<RQ>>) -> Self {
        DecisionRequestBatch {
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

impl<DAD, PM> From<PartialDecisionInformation<DAD, PM>>
    for (MaybeVec<DAD>, MaybeVec<Arc<StoredMessage<PM>>>)
{
    fn from(value: PartialDecisionInformation<DAD, PM>) -> Self {
        (value.message_partial_info, value.messages)
    }
}
