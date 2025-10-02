use either::Either;
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use atlas_common::error::*;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::PProof;
use atlas_core::ordering_protocol::networking::serialize::{
    OrderProtocolProof, OrderingProtocolMessage,
};
use atlas_logging_core::decision_log::serialize::OrderProtocolLog;

#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
// Checkout https://serde.rs/attr-bound.html as to why we are using this
#[serde(bound = "")]
pub struct DecisionLog<RQ, OP, POP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OP>,
{
    last_exec: Option<SeqNo>,
    decided: Vec<PProof<RQ, OP, POP>>,
}

impl<RQ, OP, POP> Default for DecisionLog<RQ, OP, POP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OP>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<RQ, OP, POP> DecisionLog<RQ, OP, POP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OP>,
{
    pub fn new() -> Self {
        Self {
            last_exec: None,
            decided: vec![],
        }
    }

    /// Initialize a decision log from a given vector of proofs
    pub fn from_decided(last_exec: SeqNo, proofs: Vec<PProof<RQ, OP, POP>>) -> Self {
        Self {
            last_exec: Some(last_exec),
            decided: proofs,
        }
    }

    /// Assemble a decision log from a vector of proofs
    pub fn from_proofs(mut proofs: Vec<PProof<RQ, OP, POP>>) -> Self {
        proofs.sort_by(|a, b| a.sequence_number().cmp(&b.sequence_number()).reverse());

        let last_decided = proofs.last().map(|proof| proof.sequence_number());

        Self {
            last_exec: last_decided,
            decided: proofs,
        }
    }

    pub fn from_ordered_proofs(proofs: Vec<PProof<RQ, OP, POP>>) -> Self {
        let last_decided = proofs.last().map(|proof| proof.sequence_number());

        Self {
            last_exec: last_decided,
            decided: proofs,
        }
    }

    /// Returns the sequence number of the last executed batch of client
    /// requests, assigned by the conesensus layer.
    pub fn last_execution(&self) -> Option<SeqNo> {
        self.last_exec
    }

    /// Get all of the decided proofs in this decisionn log
    pub fn proofs(&self) -> &[PProof<RQ, OP, POP>] {
        &self.decided[..]
    }

    /// Append a proof to the end of the log. Assumes all prior checks have been done
    pub(crate) fn append_proof(&mut self, proof: PProof<RQ, OP, POP>) -> Result<()> {
        self.last_exec = Some(proof.sequence_number());

        self.decided.push(proof);

        Ok(())
    }

    //TODO: Maybe make these data structures a BTreeSet so that the messages are always ordered
    //By their seq no? That way we cannot go wrong in the ordering of messages.
    #[allow(dead_code)]
    pub(crate) fn finished_quorum_execution(
        &mut self,
        proof: &PProof<RQ, OP, POP>,
        seq_no: SeqNo,
    ) -> Result<()> {
        self.last_exec.replace(seq_no);

        self.decided.push(proof.clone());

        Ok(())
    }

    /// Get a proof of a given sequence number
    pub(crate) fn get_proof(&self, seq: SeqNo) -> Option<PProof<RQ, OP, POP>> {
        if let Some(first_seq) = self.first_seq() {
            match seq.index(first_seq) {
                Either::Left(_) => None,
                Either::Right(index) => {
                    if index < self.decided.len() {
                        Some(self.decided[index].clone())
                    } else {
                        None
                    }
                }
            }
        } else {
            None
        }
    }

    /// Returns the proof of the last executed consensus
    /// instance registered in this `DecisionLog`.
    pub fn last_decision(&self) -> Option<PProof<RQ, OP, POP>> {
        self.decided.last().map(|p| (*p).clone())
    }

    /// Returns a reference to the last executed consensus instance
    /// in the decision log
    pub fn last_decision_ref(&self) -> Option<&PProof<RQ, OP, POP>> {
        self.decided.last()
    }

    /// Clear the decision log until the given sequence number
    pub(crate) fn clear_until_seq(&mut self, seq_no: SeqNo) -> usize {
        let net_decided = Vec::new();

        let mut prev_decided = std::mem::replace(&mut self.decided, net_decided);

        while let Some(decision) = prev_decided.pop() {
            if decision.sequence_number() >= seq_no {
                self.decided.push(decision);
            } else {
                break;
            }
        }

        self.decided.reverse();

        prev_decided
            .into_iter()
            .map(|item| item.contained_messages())
            .sum()
    }

    pub(crate) fn into_proofs(self) -> Vec<PProof<RQ, OP, POP>> {
        self.decided
    }
}

impl<RQ, OP, POP> Orderable for DecisionLog<RQ, OP, POP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OP>,
{
    fn sequence_number(&self) -> SeqNo {
        self.last_exec.unwrap_or(SeqNo::ZERO)
    }
}

impl<RQ, OP, POP> OrderProtocolLog for DecisionLog<RQ, OP, POP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OP>,
{
    fn first_seq(&self) -> Option<SeqNo> {
        self.decided
            .first()
            .map(|decided| decided.sequence_number())
    }
}

impl<RQ, OP, POP> Clone for DecisionLog<RQ, OP, POP>
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
    POP: PersistentOrderProtocolTypes<RQ, OP>,
{
    fn clone(&self) -> Self {
        DecisionLog {
            last_exec: self.last_exec,
            decided: self.decided.clone(),
        }
    }
}
