pub mod network;

use std::collections::{BTreeSet, VecDeque};


use atlas_common::crypto::hash::Digest;
use tracing::{debug, warn};

use atlas_common::node_id::NodeId;
use atlas_communication::message::{Header, StoredMessage};

use crate::message::{OrderedBCastMessage};
use crate::threshold_crypto::ordered_bcast::network::OrderedBCastNode;

/// Ordering of messages received our of order
struct PendingOrderedBCastMessages<T> {
    // Pending order messages, to be processed when ready
    pending_order: VecDeque<StoredMessage<OrderedBCastMessage<T>>>,

    // Pending order vote messages, to be processed when ready
    pending_order_vote: VecDeque<StoredMessage<OrderedBCastMessage<T>>>,
}

/// In an ordered fashion, we broadcast a message to all members of the quorum
///
/// Ordered broadcast is a 3 phase protocol, where
/// the first phase serves to collect all the messages,
/// the second phase serves for the leader to propagate his decided order to the quorum
/// and the third phase serves for all members to vote on the decided order
///
/// This is based on the PBFT protocol.
/// with the difference that we do not need two rounds of voting,
/// since we don't need to persist them across view changes, so the commit phase is removed
///
/// The ordered broadcast protocol always chooses the leader to be the node which starts
/// the protocol
pub(crate) struct OrderedBroadcast<T> {
    // Our ID
    our_id: NodeId,
    // The leader of this ordered broadcast
    leader: NodeId,
    // The threshold amount of nodes needed to assure security
    threshold: usize,
    // The members that are partaking in this broadcast
    members: Vec<NodeId>,
    // The pending messages that we still have not processed
    pending_message: PendingOrderedBCastMessages<T>,
    // The current phase of this broadcast
    phase: OrderedBCastPhase<T>,
}

/// The inner synchronous broadcast phase phases
pub(crate) enum OrderedBCastPhase<T> {
    // We are currently collecting the various possible messages
    // Which will need to be ordered and broadcast in a given order
    // By the leader
    CollectionPhase(Vec<T>, BTreeSet<NodeId>),
    // We are currently awaiting the order from the leader
    // In order to begin voting on it
    // We store all the values that we have collected
    AwaitingOrder,
    // Once the leader has decided on the order, we are in the voting phase
    // And we wait for the other nodes to vote on the order
    VotingPhase(Vec<T>, BTreeSet<NodeId>),
    // Once we have received enough votes, the order is decided
    // And the broadcast is complete
    Done(Vec<T>),
}

impl<T> OrderedBroadcast<T>
where
    T: Clone,
{
    /// Initialize a new ordered broadcast instance
    pub fn init_ordered_bcast(
        our_id: NodeId,
        leader: NodeId,
        threshold: usize,
        members: Vec<NodeId>,
    ) -> Self {
        Self {
            our_id,
            leader,
            threshold,
            members,
            pending_message: PendingOrderedBCastMessages {
                pending_order: Default::default(),
                pending_order_vote: Default::default(),
            },
            phase: OrderedBCastPhase::CollectionPhase(Vec::new(), BTreeSet::new()),
        }
    }

    pub fn is_ready(&self) -> bool {
        if let OrderedBCastPhase::Done(_) = &self.phase {
            true
        } else {
            false
        }
    }

    pub(crate) fn handle_message<NT>(
        &mut self,
        node: &NT,
        header: Header,
        bcast_message: OrderedBCastMessage<T>,
    ) where
        NT: OrderedBCastNode<T>,
    {
        match &mut self.phase {
            OrderedBCastPhase::CollectionPhase(received, voted) => {
                match bcast_message {
                    OrderedBCastMessage::Value(value) => {
                        if voted.insert(header.from()) {
                            received.push(value);

                            if voted.len() >= self.threshold {
                                let received_cpy = received.clone();

                                // We have received enough messages to begin the ordering phase
                                self.phase = OrderedBCastPhase::AwaitingOrder;

                                if self.our_id == self.leader {
                                    // We are the leader, so we can decide on the order
                                    // And broadcast it to the other nodes
                                    self.decide_and_bcast_order(node, received_cpy);
                                }
                            }
                        } else {
                            warn!("Received duplicate message from node {:?}", header.from());
                        }
                    }
                    _ => {
                        self.pending_message.queue_message(header, bcast_message);
                    }
                }
            }
            OrderedBCastPhase::AwaitingOrder if header.from() == self.leader => match bcast_message
            {
                OrderedBCastMessage::Value(_) => {}
                OrderedBCastMessage::Order(order) => {
                    self.phase = OrderedBCastPhase::VotingPhase(order, BTreeSet::new());

                    self.vote_on_order(node, header.digest().clone());
                }
                OrderedBCastMessage::OrderVote(_) => {
                    self.pending_message.queue_message(header, bcast_message);
                }
            },
            OrderedBCastPhase::AwaitingOrder => {
                warn!(
                    "Received message from non-leader node {:?} while awaiting order",
                    header.from()
                );
            }
            OrderedBCastPhase::VotingPhase(order, received_votes) => {
                if received_votes.insert(header.from()) {
                    self.phase = OrderedBCastPhase::Done(std::mem::replace(order, Vec::new()));
                } else {
                    warn!("Received duplicate vote from node {:?}", header.from());
                }
            }
            OrderedBCastPhase::Done(_order) => {
                debug!(
                    "Received message from node {:?} after broadcast was done",
                    header.from()
                );
            }
        }
    }

    fn decide_and_bcast_order<NT>(&mut self, node: &NT, order: Vec<T>)
    where
        NT: OrderedBCastNode<T>,
    {
        let _ = node.broadcast_ordered_bcast_message(
            OrderedBCastMessage::Order(order),
            self.members.clone().into_iter(),
        );
    }

    fn vote_on_order<NT>(&mut self, node: &NT, digest: Digest)
    where
        NT: OrderedBCastNode<T>,
    {
        let _ = node.broadcast_ordered_bcast_message(
            OrderedBCastMessage::OrderVote(digest),
            self.members.clone().into_iter(),
        );
    }

    pub fn finish(self) -> Vec<T> {
        if let OrderedBCastPhase::Done(order) = self.phase {
            order
        } else {
            unreachable!("How can we finish a broadcast that is not done?")
        }
    }
}

impl<T> PendingOrderedBCastMessages<T> {
    // Query a message for latter processing
    fn queue_message(&mut self, header: Header, bcast_message: OrderedBCastMessage<T>) {
        match bcast_message {
            OrderedBCastMessage::Order(_) => {
                self.pending_order
                    .push_back(StoredMessage::new(header.clone(), bcast_message));
            }
            OrderedBCastMessage::OrderVote(_) => {
                self.pending_order_vote
                    .push_back(StoredMessage::new(header.clone(), bcast_message));
            }
            _ => unreachable!(
                "How can we query a message that is supposed to be processed immediately?"
            ),
        }
    }
}
