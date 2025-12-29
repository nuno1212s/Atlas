use anyhow::Context;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use atlas_smr_application::TExecutionHandle;
use atlas_smr_application::preemptive_execution::TPreemptiveExecutionHandle;
use std::time::Instant;

pub enum PreemptiveExecutionRequest<O> {
    PollStateChannel,

    CatchUp(MaybeVec<UpdateBatch<O>>),

    PreemptiveUpdate((UpdateBatch<O>, Instant)),

    UpdateFinalized(SeqNo),

    UpdateFinalizedAndGetAppstate(SeqNo),

    ExecuteUnordered(UnorderedUpdateBatch<O>),

    Read(NodeId),
}

pub struct PreemptiveExecutorHandler<RQ> {
    e_tx: ChannelSyncTx<PreemptiveExecutionRequest<RQ>>,
    request_rx: ChannelSyncRx<PreemptiveExecutionRequest<RQ>>,
}

impl<RQ> TExecutionHandle<RQ> for PreemptiveExecutorHandler<RQ>
where
    RQ: Send,
{
    fn poll_state_channel(&self) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::PollStateChannel)
            .context("Failed to send PollStateChannel request to executor")
    }

    fn catch_up_to_quorum(
        &self,
        requests: MaybeVec<UpdateBatch<RQ>>,
    ) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::CatchUp(requests))
            .context("Failed to send CatchUp request to executor")
    }

    fn queue_unordered(
        &self,
        requests: UnorderedUpdateBatch<RQ>,
    ) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::ExecuteUnordered(requests))
            .context("Failed to send ExecuteUnordered request to executor")
    }
}

impl<RQ> TPreemptiveExecutionHandle<RQ> for PreemptiveExecutorHandler<RQ>
where
    RQ: Send,
{
    fn queue_preemptive_update(&self, batch: UpdateBatch<RQ>) -> atlas_common::error::Result<()> {
        let now = Instant::now();
        self.e_tx
            .send(PreemptiveExecutionRequest::PreemptiveUpdate((batch, now)))
            .context("Failed to send PreemptiveUpdate request to executor")
    }

    fn queue_update_finalized(&self, seq: SeqNo) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::UpdateFinalized(seq))
            .context("Failed to send UpdateFinalized request to executor")
    }

    fn queue_update_finalized_and_get_appstate(
        &self,
        seq: SeqNo,
    ) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstate(
                seq,
            ))
            .context("Failed to send UpdateFinalizedAndGetAppstate request to executor")
    }
}

impl<RQ> Clone for PreemptiveExecutorHandler<RQ> {
    fn clone(&self) -> Self {
        Self {
            e_tx: self.e_tx.clone(),
            request_rx: self.request_rx.clone(),
        }
    }
}
