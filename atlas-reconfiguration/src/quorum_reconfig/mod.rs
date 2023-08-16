use std::sync::Arc;
use futures::future::join_all;
use atlas_common::channel::OneShotRx;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use crate::message::{NodeTriple, QuorumEnterRejectionReason, QuorumEnterResponse};

pub mod node_types;

pub type QuorumPredicate = fn(QuorumView, NodeTriple) -> OneShotRx<Option<QuorumEnterRejectionReason>>;

/// The current view of nodes in the network, as in which of them
/// are currently partaking in the consensus
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct QuorumView {
    sequence_number: SeqNo,

    quorum_members: Vec<NodeId>,
}

impl Orderable for QuorumView {
    fn sequence_number(&self) -> SeqNo {
        self.sequence_number
    }
}

impl QuorumView {
    pub fn empty() -> Self {
        QuorumView {
            sequence_number: SeqNo::ZERO,
            quorum_members: Vec::new(),
        }
    }

    pub fn with_bootstrap_nodes(bootstrap_nodes: Vec<NodeId>) -> Self {
        QuorumView {
            sequence_number: SeqNo::ZERO,
            quorum_members: bootstrap_nodes,
        }
    }

    pub fn next_with_added_node(&self, node_id: NodeId) -> Self {
        QuorumView {
            sequence_number: self.sequence_number.next(),
            quorum_members: {
                let mut members = self.quorum_members.clone();
                members.push(node_id);
                members
            },
        }
    }

    pub fn sequence_number(&self) -> SeqNo {
        self.sequence_number
    }

    pub fn quorum_members(&self) -> &Vec<NodeId> {
        &self.quorum_members
    }
}