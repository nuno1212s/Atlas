use std::sync::Arc;
use futures::future::join_all;
use atlas_common::channel::OneShotRx;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use crate::message::{NodeTriple, QuorumEnterRejectionReason, QuorumEnterResponse, QuorumNodeJoinApproval};

pub mod node_types;

pub type QuorumPredicate =
fn(QuorumView, NodeTriple) -> OneShotRx<Option<QuorumEnterRejectionReason>>;

pub struct QuorumNode {
    node_id: NodeId,
    current_network_view: QuorumView,

    /// Predicates that must be satisfied for a node to be allowed to join the quorum
    predicates: Vec<QuorumPredicate>,
}

/// The current view of nodes in the network, as in which of them
/// are currently partaking in the consensus
#[derive(Clone, Debug)]
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

impl QuorumNode {
    pub fn empty_quorum_view(node_id: NodeId) -> Self {
        QuorumNode {
            node_id,
            current_network_view: QuorumView::empty(),
            predicates: vec![],
        }
    }

    pub fn install_network_view(&mut self, network_view: QuorumView) {
        self.current_network_view = network_view;
    }

    /// Get the current network view
    pub fn network_view(&self) -> QuorumView {
        self.current_network_view.clone()
    }

    /// Are we a member of the current quorum
    pub fn is_quorum_member(&self) -> bool {
        self.current_network_view
            .quorum_members
            .contains(&self.node_id)
    }

    /// Can a given node join the quorum
    pub async fn can_node_join_quorum(self: Arc<Self>, node_id: NodeTriple) -> QuorumEnterResponse {
        if !self.is_quorum_member() {
            return QuorumEnterResponse::Rejected(
                QuorumEnterRejectionReason::NodeIsNotQuorumParticipant,
            );
        }

        let mut results = Vec::with_capacity(self.predicates.len());

        for x in &self.predicates {
            let rx = x(self.current_network_view.clone(), node_id.clone());

            results.push(rx);
        }

        let results = join_all(results.into_iter()).await;

        for join_result in results {
            if let Some(reason) = join_result.unwrap() {
                return QuorumEnterResponse::Rejected(reason);
            }
        }

        return QuorumEnterResponse::Successful(QuorumNodeJoinApproval::new(
            self.current_network_view.sequence_number(),
            node_id.node_id(),
            self.node_id,
        ));
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

impl Clone for QuorumNode {
    fn clone(&self) -> Self {
        QuorumNode {
            node_id: self.node_id,
            current_network_view: self.current_network_view.clone(),
            predicates: self.predicates.clone(),
        }
    }
}
