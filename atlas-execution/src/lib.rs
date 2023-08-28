use atlas_common::channel::ChannelSyncTx;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use crate::app::{Reply, Request, UnorderedBatch, UpdateBatch};
use crate::serialize::ApplicationData;
use std::time::Instant;

pub mod serialize;
pub mod app;
pub mod system_params;
pub mod state;

pub enum ExecutionRequest<O> {
    PollStateChannel,

    CatchUp(Vec<O>),

    // update the state of the service
    Update((UpdateBatch<O>, Instant)),
    // same as above, and include the application state
    // in the reply, used for local checkpoints
    UpdateAndGetAppstate((UpdateBatch<O>, Instant)),

    //Execute an un ordered batch of requests
    ExecuteUnordered(UnorderedBatch<O>),

    // read the state of the service
    Read(NodeId),
}

/// Represents a handle to the client request executor.
pub struct ExecutorHandle<D: ApplicationData> {
    e_tx: ChannelSyncTx<ExecutionRequest<D::Request>>,
}

impl<D: ApplicationData> ExecutorHandle<D>
{

    pub fn new(tx: ChannelSyncTx<ExecutionRequest<D::Request>>) -> Self {
        ExecutorHandle { e_tx: tx }
    }

    /// Sets the current state of the execution layer to the given value.
    pub fn poll_state_channel(&self) -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::PollStateChannel)
            .simple(ErrorKind::Executable)
    }

    pub fn catch_up_to_quorum(&self, requests: Vec<D::Request>) -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::CatchUp(requests))
            .simple(ErrorKind::Executable)
    }

    /// Queues a batch of requests `batch` for execution.
    pub fn queue_update(&self, batch: UpdateBatch<D::Request>)
                        -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::Update((batch, Instant::now())))
            .simple(ErrorKind::Executable)
    }

    /// Queues a batch of unordered requests for execution
    pub fn queue_update_unordered(&self, requests: UnorderedBatch<D::Request>)
                                  -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::ExecuteUnordered(requests))
            .simple(ErrorKind::Executable)
    }

    /// Same as `queue_update()`, additionally reporting the serialized
    /// application state.
    ///
    /// This is useful during local checkpoints.
    pub fn queue_update_and_get_appstate(
        &self,
        batch: UpdateBatch<D::Request>,
    ) -> Result<()> {
        self.e_tx
            .send(ExecutionRequest::UpdateAndGetAppstate((batch, Instant::now())))
            .simple(ErrorKind::Executable)
    }
}

impl<D: ApplicationData> Clone for ExecutorHandle<D> {
    fn clone(&self) -> Self {
        let e_tx = self.e_tx.clone();
        Self { e_tx }
    }
}