use std::ops::Deref;

use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::Orderable;
use atlas_communication::message::StoredMessage;
use atlas_core::executor::DecisionExecutorHandle;
use atlas_core::messages::SessionBased;
use atlas_core::ordering_protocol::BatchedDecision;
use atlas_smr_application::app::{UnorderedBatch, UpdateBatch};
use atlas_smr_application::ExecutorHandle;

use crate::SMRRawReq;

pub trait StateExecutorTrait {
    fn start_polling_state(&self) -> Result<()>;
}

#[derive(Clone, Copy)]
pub enum RequestType {
    Ordered,
    Unordered,
}

/// Trait for a network node capable of sending replies to clients
pub trait ReplyNode<RP>: Send + Sync {
    fn send(&self, reply_type: RequestType, reply: RP, target: NodeId, flush: bool) -> Result<()>;

    fn send_signed(
        &self,
        reply_type: RequestType,
        reply: RP,
        target: NodeId,
        flush: bool,
    ) -> Result<()>;

    fn broadcast(
        &self,
        reply_type: RequestType,
        reply: RP,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;

    fn broadcast_signed(
        &self,
        reply_type: RequestType,
        reply: RP,
        targets: impl Iterator<Item = NodeId>,
    ) -> std::result::Result<(), Vec<NodeId>>;
}

pub struct WrappedExecHandle<R>(pub ExecutorHandle<R>);

impl<R> Clone for WrappedExecHandle<R> {
    fn clone(&self) -> Self {
        WrappedExecHandle(self.0.clone())
    }
}

impl<R> WrappedExecHandle<R> {
    pub fn transform_update_batch(decision: BatchedDecision<SMRRawReq<R>>) -> UpdateBatch<R> {
        let mut update_batch =
            UpdateBatch::new_with_cap(decision.sequence_number(), decision.len());

        decision.into_inner().into_iter().for_each(|request| {
            let (header, message) = request.into_inner();

            update_batch.add(
                header.from(),
                message.session_number(),
                message.sequence_number(),
                message.into_inner_operation(),
            );
        });

        update_batch
    }

    fn transform_unordered_batch(decision: Vec<StoredMessage<SMRRawReq<R>>>) -> UnorderedBatch<R> {
        let mut update_batch = UnorderedBatch::new_with_cap(decision.len());

        decision.into_iter().for_each(|request| {
            let (header, message) = request.into_inner();

            update_batch.add(
                header.from(),
                message.session_number(),
                message.sequence_number(),
                message.into_inner_operation(),
            );
        });

        update_batch
    }
}

impl<R> DecisionExecutorHandle<SMRRawReq<R>> for WrappedExecHandle<R>
where
    R: Send + 'static,
{
    fn catch_up_to_quorum(&self, requests: MaybeVec<BatchedDecision<SMRRawReq<R>>>) -> Result<()> {
        let requests: MaybeVec<_> = requests
            .into_iter()
            .map(Self::transform_update_batch)
            .collect();

        self.0.catch_up_to_quorum(requests)
    }

    fn queue_update(&self, batch: BatchedDecision<SMRRawReq<R>>) -> Result<()> {
        self.0.queue_update(Self::transform_update_batch(batch))
    }

    fn queue_update_unordered(&self, requests: Vec<StoredMessage<SMRRawReq<R>>>) -> Result<()> {
        self.0
            .queue_update_unordered(Self::transform_unordered_batch(requests))
    }
}

impl<R> Deref for WrappedExecHandle<R> {
    type Target = ExecutorHandle<R>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
