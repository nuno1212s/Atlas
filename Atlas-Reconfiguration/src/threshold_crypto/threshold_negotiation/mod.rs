use std::collections::BTreeMap;
use std::sync::Arc;

use tracing::warn;

use atlas_common::crypto::threshold_crypto::thold_crypto::dkg::{
    Ack, DKGParams, DealerPart, DistributedKeyGenerator,
};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_communication::message::{Header, StoredMessage};


use crate::message::{
    OrderedBCastMessage, ThresholdDKGArgs, ThresholdMessages,
};
use crate::threshold_crypto::network::ThresholdNetwork;
use crate::threshold_crypto::ordered_bcast::OrderedBroadcast;
use crate::threshold_crypto::threshold_negotiation::network_bridge::{
    DkgAckSendNodeRef, DkgDealerSendNodeRef,
};

mod network_bridge;

/// The ordered dealer parts, decided by the leader
struct OrderedDealerParts(Vec<(usize, DealerPart)>);

/// The ordered Ack messages, decided by the leader and
/// voted for by the other nodes in the quorum
struct OrderedAcks(Vec<(usize, Ack)>);

pub type AckOrderedBCastMessage = OrderedBCastMessage<Ack>;
pub type DealerPartOrderedBCastMessage = OrderedBCastMessage<DealerPart>;

/// Pending messages that we have not yet processed
pub struct PendingMessages {
    // The pending dealer part ordered broadcast messages
    pending_dealer_messages: Vec<StoredMessage<OrderedBCastMessage<DealerPart>>>,
    // The pending ack part ordered broadcast messages
    pending_ack_messages: Vec<StoredMessage<OrderedBCastMessage<Vec<Ack>>>>,
}

/// The current state of our joining protocol.
/// We want to achieve a common order in a byzantine
/// fault tolerant scenario
enum JoiningThresholdReplicaState {
    InitLeader,
    // Initial state, where we have done nothing at all
    Init,
    // exchanging dealer parts and attaining a global ordering for them
    PartExchange(OrderedBroadcast<DealerPart>),
    // Exchanging acks and attaining a global ordering for them
    // We order entire vec of acks as each of the users produces a vec of acks (one for
    // Each received dealer part)
    AckExchange(OrderedBroadcast<Vec<Ack>>),
}

/// A node that is currently attempting to join the network
struct JoiningThresholdReplica {
    our_id: NodeId,
    // The leader of this DKG protocol
    // This is who will be used as the leader in the ordered broadcast protocols
    leader: NodeId,
    // The quorum partaking in this
    quorum: Vec<NodeId>,
    // Our dealer part
    dealer_part: DealerPart,

    threshold: usize,

    // The map of each participating node, along with the index of the node in
    // This join protocol
    participating_nodes: BTreeMap<NodeId, usize>,

    // The distributed key generator
    dkg: DistributedKeyGenerator,

    // The current enum state of the reconfiguration threshold key generation
    // Join protocol
    current_state: JoiningThresholdReplicaState,
    // Messages that we received while we were in the middle of another
    // Incompatible phase
    pending_messages: PendingMessages,
}

impl JoiningThresholdReplica {
    /// Initialize a distributed key generation protocol
    pub fn initialize_dkg_protocol(
        our_id: NodeId,
        quorum: Vec<NodeId>,
        threshold: usize,
    ) -> Result<Self> {
        let mut participating_nodes: BTreeMap<NodeId, usize> = Default::default();

        quorum.iter().enumerate().for_each(|(id, node)| {
            participating_nodes.insert(*node, id + 1);
        });

        let params = DKGParams::new(quorum.len(), threshold);

        let participating_node_id = participating_nodes
            .get(&our_id)
            .cloned()
            .expect("Failed to get our own transaction ID?");

        let (dkg, dealer) = DistributedKeyGenerator::new(params, participating_node_id)?;

        Ok(Self {
            our_id,
            leader: our_id,
            quorum,
            dealer_part: dealer,
            threshold,
            participating_nodes,
            dkg,
            current_state: JoiningThresholdReplicaState::Init,
            pending_messages: PendingMessages {
                pending_dealer_messages: Vec::new(),
                pending_ack_messages: Vec::new(),
            },
        })
    }

    /// Initialize the protocol from an init message
    pub fn from_init_message(our_id: NodeId, dkg_args: ThresholdDKGArgs) -> Result<Self> {
        Self::initialize_dkg_protocol(our_id, dkg_args.quorum, dkg_args.threshold)
    }

    fn iterate<NT>(&mut self, node: Arc<NT>) -> Result<()>
    where
        NT: ThresholdNetwork + 'static,
    {
        match &mut self.current_state {
            JoiningThresholdReplicaState::InitLeader => {
                let trigger_dkg = ThresholdMessages::TriggerDKG(ThresholdDKGArgs::init_args(
                    self.quorum.clone(),
                    self.threshold,
                    self.our_id,
                ));

                let _ =
                    node.broadcast_threshold_message(trigger_dkg, self.quorum.clone().into_iter());

                self.current_state = JoiningThresholdReplicaState::Init;
            }
            JoiningThresholdReplicaState::Init => {
                let ordered_bcast = OrderedBroadcast::<DealerPart>::init_ordered_bcast(
                    self.our_id,
                    self.leader,
                    self.threshold,
                    self.quorum.clone(),
                );

                let reconfig_msg_type = ThresholdMessages::DkgDealer(OrderedBCastMessage::Value(
                    self.dealer_part.clone(),
                ));

                let _ = node.broadcast_threshold_message(
                    reconfig_msg_type,
                    self.quorum.clone().into_iter(),
                );

                self.current_state = JoiningThresholdReplicaState::PartExchange(ordered_bcast);
            }
            JoiningThresholdReplicaState::PartExchange(dealer_part_exchange)
                if dealer_part_exchange.is_ready() =>
            {
                let ordered_bcast = OrderedBroadcast::<Vec<Ack>>::init_ordered_bcast(
                    self.our_id,
                    self.leader,
                    self.threshold,
                    self.quorum.clone(),
                );

                let part_exchange = std::mem::replace(
                    &mut self.current_state,
                    JoiningThresholdReplicaState::AckExchange(ordered_bcast),
                );

                let broadcast = part_exchange.unwrap_ordered_bcast_dealer();

                let ordered_dealer_parts = broadcast.finish();

                let acks = ordered_dealer_parts
                    .into_iter()
                    .map(|dealer_part| self.dkg.handle_part(dealer_part.author(), dealer_part))
                    .collect::<Result<Vec<Ack>>>()?;

                let thold_messages = ThresholdMessages::DkgAck(OrderedBCastMessage::Value(acks));

                let _ = node
                    .broadcast_threshold_message(thold_messages, self.quorum.clone().into_iter());
            }
            JoiningThresholdReplicaState::AckExchange(ack_part_exchange)
                if ack_part_exchange.is_ready() =>
            {
                todo!()
            }
            _ => {
                // Nothing is necessary
            }
        }

        Ok(())
    }

    fn handle_message<NT>(&mut self, node: &NT, header: Header, message: ThresholdMessages)
    where
        NT: ThresholdNetwork + 'static,
    {
        match &mut self.current_state {
            JoiningThresholdReplicaState::InitLeader => {
                warn!("Received message while in InitLeader state (How is this possible if we are the leader and we ?)");
            }
            JoiningThresholdReplicaState::Init => {}
            JoiningThresholdReplicaState::PartExchange(exchange) => match message {
                ThresholdMessages::TriggerDKG(_dk) => {}
                ThresholdMessages::DkgDealer(dealer_part) => {
                    exchange.handle_message(&DkgDealerSendNodeRef::from(node), header, dealer_part);
                }
                ThresholdMessages::DkgAck(ack_parts) => self
                    .pending_messages
                    .queue_message(header, ThresholdMessages::DkgAck(ack_parts)),
            },
            JoiningThresholdReplicaState::AckExchange(ack_part) => match message {
                ThresholdMessages::TriggerDKG(_dk) => {}
                ThresholdMessages::DkgDealer(dealer_part) => self
                    .pending_messages
                    .queue_message(header, ThresholdMessages::DkgDealer(dealer_part)),
                ThresholdMessages::DkgAck(ack_parts) => {
                    ack_part.handle_message(&DkgAckSendNodeRef::from(node), header, ack_parts)
                }
            },
        }
    }
}

impl PendingMessages {
    fn queue_message(&mut self, header: Header, message: ThresholdMessages) {
        match message {
            ThresholdMessages::DkgDealer(dealer_part) => {
                self.pending_dealer_messages
                    .push(StoredMessage::new(header, dealer_part));
            }
            ThresholdMessages::DkgAck(ack) => {
                self.pending_ack_messages
                    .push(StoredMessage::new(header, ack));
            }
            _ => unreachable!("Invalid message type for pending messages"),
        }
    }
}

impl JoiningThresholdReplicaState {
    fn unwrap_ordered_bcast_dealer(self) -> OrderedBroadcast<DealerPart> {
        match self {
            JoiningThresholdReplicaState::PartExchange(ordered_bcast) => ordered_bcast,
            _ => unreachable!("Invalid state for ordered broadcast"),
        }
    }

    fn unwrap_ordered_bcast_acks(self) -> OrderedBroadcast<Vec<Ack>> {
        match self {
            JoiningThresholdReplicaState::AckExchange(ordered_bcast) => ordered_bcast,
            _ => unreachable!("Invalid state for ordered broadcast"),
        }
    }
}
