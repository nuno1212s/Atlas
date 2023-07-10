use std::sync::Arc;
use futures::future::join_all;
use atlas_common::channel::OneShotRx;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use crate::message::{NodeTriple, QuorumEnterRejectionReason, QuorumEnterResponse, QuorumNodeJoinResponse};

pub type QuorumPredicate =
fn(Arc<QuorumNode>, NodeTriple) -> OneShotRx<Option<QuorumEnterRejectionReason>>;

pub struct QuorumNode {
    node_id: NodeId,
    current_network_view: NetworkView,

    /// Predicates that must be satisfied for a node to be allowed to join the quorum
    predicates: Vec<QuorumPredicate>,
}

/// The current view of nodes in the network, as in which of them
/// are currently partaking in the consensus
#[derive(Clone)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct NetworkView {
    sequence_number: SeqNo,

    quorum_members: Vec<NodeId>,
}

impl Orderable for NetworkView {
    fn sequence_number(&self) -> SeqNo {
        self.sequence_number
    }
}

impl QuorumNode {
    pub fn empty_quorum_node(node_id: NodeId) -> Self {
        QuorumNode {
            node_id,
            current_network_view: NetworkView::empty(),
            predicates: vec![],
        }
    }

    pub fn install_network_view(&mut self, network_view: NetworkView) {
        self.current_network_view = network_view;
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
            let rx = x(self.clone(), node_id.clone());

            results.push(rx);
        }

        let results = join_all(results.into_iter()).await;

        for join_result in results {
            if let Some(reason) = join_result.unwrap() {
                return QuorumEnterResponse::Rejected(reason);
            }
        }

        return QuorumEnterResponse::Successful(QuorumNodeJoinResponse::new(
            self.current_network_view.sequence_number(),
            node_id.node_id(),
            self.node_id,
        ));
    }
}
