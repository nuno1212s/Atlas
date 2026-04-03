use anyhow::Context;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::node_id::NodeId;
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use atlas_smr_application::TExecutionHandle;
use atlas_smr_application::deterministic_execution::TDeterministicExecutionHandle;
use std::time::Instant;

pub enum ExecutionRequest<O> {
    // Poll the state channel
    // As we have an incoming state update
    PollStateChannel,

    // Catch up to the current execution by
    // Executing the given requests
    CatchUp(MaybeVec<UpdateBatch<O>>),

    // update the state of the service
    Update((UpdateBatch<O>, Instant)),
    // same as above, and include the application state
    // in the reply, used for local checkpoints
    UpdateAndGetAppstate((UpdateBatch<O>, Instant)),

    //Execute an un ordered batch of requests
    ExecuteUnordered(UnorderedUpdateBatch<O>),

    // read the state of the service
    Read(NodeId),
}

/// Represents a handle to the client request execution.
pub struct ExecutorHandle<RQ> {
    e_tx: ChannelSyncTx<ExecutionRequest<RQ>>,
    request_rx: ChannelSyncRx<ExecutionRequest<RQ>>,
}

impl<RQ> ExecutorHandle<RQ> {
    /// Creates a new `ExecutorHandle` with the given execution channel sender.
    pub fn new(
        e_tx: ChannelSyncTx<ExecutionRequest<RQ>>,
        request_rx: ChannelSyncRx<ExecutionRequest<RQ>>,
    ) -> Self {
        Self { e_tx, request_rx }
    }

    /// Returns the execution request receiver channel.
    pub fn get_request_receiver(&self) -> &ChannelSyncRx<ExecutionRequest<RQ>> {
        &self.request_rx
    }
}

impl<RQ> TExecutionHandle<RQ> for ExecutorHandle<RQ>
where
    RQ: Send,
{
    fn poll_state_channel(&self) -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::PollStateChannel)
            .context("Failed to place poll order into execution channel")
    }

    fn catch_up_to_quorum(&self, requests: MaybeVec<UpdateBatch<RQ>>) -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::CatchUp(requests))
            .context("Failed to place catch up order into execution channel")
    }

    fn queue_unordered(&self, requests: UnorderedUpdateBatch<RQ>) -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::ExecuteUnordered(requests))
            .context("Failed to place unordered update order into execution channel")
    }
}

impl<RQ> TDeterministicExecutionHandle<RQ> for ExecutorHandle<RQ>
where
    RQ: Send,
{
    fn queue_update(&self, batch: UpdateBatch<RQ>) -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::Update((batch, Instant::now())))
            .context("Failed to place update order into execution channel")
    }

    fn queue_update_and_get_appstate(&self, batch: UpdateBatch<RQ>) -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::UpdateAndGetAppstate((
                batch,
                Instant::now(),
            )))
            .context("Failed to place update and get appstate order into execution channel")
    }
}

impl<RQ> Clone for ExecutorHandle<RQ> {
    fn clone(&self) -> Self {
        Self {
            e_tx: self.e_tx.clone(),
            request_rx: self.request_rx.clone(),
        }
    }
}
