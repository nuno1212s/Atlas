use crate::message::{OrderedBCastMessage, ThresholdMessages};
use crate::threshold_crypto::network::ThresholdNetwork;
use crate::threshold_crypto::ordered_bcast::network::OrderedBCastNode;
use atlas_common::crypto::threshold_crypto::thold_crypto::dkg::{Ack, DealerPart};
use atlas_common::node_id::NodeId;

pub struct DkgDealerSendNode<NT> {
    node: NT,
}

impl<NT> OrderedBCastNode<DealerPart> for DkgDealerSendNode<NT>
where
    NT: ThresholdNetwork,
{
    fn send_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<DealerPart>,
        target: NodeId,
    ) -> atlas_common::error::Result<()> {
        self.node
            .send_threshold_message(ThresholdMessages::DkgDealer(message), target)
    }

    fn broadcast_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<DealerPart>,
        target: impl Iterator<Item = NodeId>,
    ) -> Result<(), Vec<NodeId>> {
        self.node
            .broadcast_threshold_message(ThresholdMessages::DkgDealer(message), target)
    }
}

pub struct DkgAckSendNode<NT> {
    node: NT,
}

impl<NT> OrderedBCastNode<Vec<Ack>> for DkgAckSendNode<NT>
where
    NT: ThresholdNetwork,
{
    fn send_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<Vec<Ack>>,
        target: NodeId,
    ) -> atlas_common::error::Result<()> {
        self.node
            .send_threshold_message(ThresholdMessages::DkgAck(message), target)
    }

    fn broadcast_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<Vec<Ack>>,
        target: impl Iterator<Item = NodeId>,
    ) -> Result<(), Vec<NodeId>> {
        self.node
            .broadcast_threshold_message(ThresholdMessages::DkgAck(message), target)
    }
}

impl<NT> From<NT> for DkgAckSendNode<NT> {
    fn from(node: NT) -> Self {
        Self { node }
    }
}

impl<NT> From<NT> for DkgDealerSendNode<NT> {
    fn from(node: NT) -> Self {
        Self { node }
    }
}

// Create the same implementations, but accepting a reference to NT instead with a 'a lifetime
pub struct DkgDealerSendNodeRef<'a, NT> {
    node: &'a NT,
}

impl<'a, NT> OrderedBCastNode<DealerPart> for DkgDealerSendNodeRef<'a, NT>
where
    NT: ThresholdNetwork,
{
    fn send_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<DealerPart>,
        target: NodeId,
    ) -> atlas_common::error::Result<()> {
        self.node
            .send_threshold_message(ThresholdMessages::DkgDealer(message), target)
    }

    fn broadcast_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<DealerPart>,
        target: impl Iterator<Item = NodeId>,
    ) -> Result<(), Vec<NodeId>> {
        self.node
            .broadcast_threshold_message(ThresholdMessages::DkgDealer(message), target)
    }
}

pub struct DkgAckSendNodeRef<'a, NT> {
    node: &'a NT,
}

impl<'a, NT> OrderedBCastNode<Vec<Ack>> for DkgAckSendNodeRef<'a, NT>
where
    NT: ThresholdNetwork,
{
    fn send_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<Vec<Ack>>,
        target: NodeId,
    ) -> atlas_common::error::Result<()> {
        self.node
            .send_threshold_message(ThresholdMessages::DkgAck(message), target)
    }

    fn broadcast_ordered_bcast_message(
        &self,
        message: OrderedBCastMessage<Vec<Ack>>,
        target: impl Iterator<Item = NodeId>,
    ) -> Result<(), Vec<NodeId>> {
        self.node
            .broadcast_threshold_message(ThresholdMessages::DkgAck(message), target)
    }
}

impl<'a, NT> From<&'a NT> for DkgAckSendNodeRef<'a, NT> {
    fn from(node: &'a NT) -> Self {
        Self { node }
    }
}

impl<'a, NT> From<&'a NT> for DkgDealerSendNodeRef<'a, NT> {
    fn from(node: &'a NT) -> Self {
        Self { node }
    }
}
