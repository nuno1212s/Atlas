pub mod executors {
    pub mod divisible_state;
    pub mod monolithic_state;
}

pub mod reply;
pub mod state_management;

use crate::SMRRawReq;
use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::Orderable;
use atlas_communication::message::StoredMessage;
use atlas_core::execution::requests::{
    IncrementableUpdateBatch, UnorderedUpdateBatch, UpdateBatch, UpdateInfo,
};
use atlas_core::execution::TDecisionExecutorHandle;
use atlas_core::messages::SessionBased;
use atlas_core::ordering_protocol::decision::BatchedDecision;
use atlas_smr_application::app::{Application, Request};
use atlas_smr_application::deterministic_execution::TDeterministicExecutionHandle;
use std::ops::Deref;
use atlas_core::execution::deterministic_execution::TDeterministicDecisionExecutorHandle;
use crate::execution::state_management::{TDeterministicExecutorStateHandle, TExecutorStateHandle};

pub trait TExecutor<A, S>
where
    A: Application<S>,
{
    type ExecutionHandle: TDeterministicExecutionHandle<Request<A, S>>;

    /// Initialize a handle and a channel to receive requests
    fn init_handle() -> Self::ExecutionHandle;
}

#[derive(Clone)]
pub struct WrappedExecHandle<E>(pub E);

impl<E> WrappedExecHandle<E> {
    pub fn transform_update_batch<RQ>(decision: BatchedDecision<SMRRawReq<RQ>>) -> UpdateBatch<RQ> {
        let update_batch = UpdateBatch::new_with_cap(decision.sequence_number(), decision.len());

        decision
            .into_inner()
            .into_iter()
            .fold(update_batch, add_stored_message_info)
    }

    fn transform_unordered_batch<RQ>(
        decision: Vec<StoredMessage<SMRRawReq<RQ>>>,
    ) -> UnorderedUpdateBatch<RQ> {
        let update_batch = UnorderedUpdateBatch::new_with_cap(decision.len());

        decision
            .into_iter()
            .fold(update_batch, add_stored_message_info)
    }
}

impl<E, RQ> TDecisionExecutorHandle<SMRRawReq<RQ>> for WrappedExecHandle<E>
where
    E: TDeterministicExecutionHandle<RQ> + Send + 'static,
{
    fn catch_up_to_quorum(&self, requests: MaybeVec<BatchedDecision<SMRRawReq<RQ>>>) -> Result<()> {
        let requests: MaybeVec<_> = requests
            .into_iter()
            .map(Self::transform_update_batch)
            .collect();

        self.0.catch_up_to_quorum(requests)
    }

    fn queue_update_unordered(&self, requests: Vec<StoredMessage<SMRRawReq<RQ>>>) -> Result<()> {
        self.0
            .queue_unordered(Self::transform_unordered_batch(requests))
    }
}

impl<E, RQ> TDeterministicDecisionExecutorHandle<SMRRawReq<RQ>> for WrappedExecHandle<E>
where
    E: TDeterministicExecutionHandle<RQ> + Send + 'static, {
    fn queue_update(&self, batch: BatchedDecision<SMRRawReq<RQ>>) -> Result<()> {
        self.0.queue_update(Self::transform_update_batch(batch))
    }
}

impl<E, RQ> TExecutorStateHandle<SMRRawReq<RQ>> for WrappedExecHandle<E>
where
    E: TDeterministicExecutionHandle<RQ> + Send + 'static,
{
    fn poll_state_channel(&self) -> Result<()> {
        self.0.poll_state_channel()
    }
}

impl<E, RQ> TDeterministicExecutorStateHandle<SMRRawReq<RQ>> for WrappedExecHandle<E>
where
    E: TDeterministicExecutionHandle<RQ> + Send + 'static,
{
    fn queue_update_and_get_appstate(&self, batch: BatchedDecision<SMRRawReq<RQ>>) -> Result<()> {
        self.0.queue_update_and_get_appstate(Self::transform_update_batch(batch))
    }
}

impl<E> Deref for WrappedExecHandle<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn add_stored_message_info<UB, R>(mut update_batch: UB, request: StoredMessage<SMRRawReq<R>>) -> UB
where
    UB: IncrementableUpdateBatch<R>,
{
    let (header, message) = request.into_inner();

    let update_info = UpdateInfo::new_session_based(
        header.from(),
        message.session_number(),
        message.sequence_number(),
    );

    update_batch.add(update_info, message.into_inner_operation());

    update_batch
}
