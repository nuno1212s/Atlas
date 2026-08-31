use anyhow::Context;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use atlas_smr_application::TExecutionHandle;
use atlas_smr_application::deterministic_execution::TDeterministicExecutionHandle;
use atlas_smr_application::preemptive_execution::TPreemptiveExecutionHandle;
use std::time::Instant;

pub enum PreemptiveExecutionRequest<O> {
    /// Instruct the executor to poll the state channel for new state updates.
    PollStateChannel,
    /// Instruct the executor to catch up to a quorum of updates.
    CatchUp(MaybeVec<UpdateBatch<O>>),
    /// A decided, finalized update batch to be executed.
    /// The Instant represents the time at which the update was originally queued for execution.
    UpdateBatch(UpdateBatch<O>, Instant),
    /// A decided, finalized update batch to be executed, and the application state to be retrieved after execution.
    /// The Instant represents the time at which the update was originally queued for execution.
    UpdateBatchAndGetAppstate(UpdateBatch<O>, Instant),
    /// A preemptive update batch to be executed.
    /// The Instant represents the time at which the update was originally queued for execution.
    PreemptiveUpdate(UpdateBatch<O>, Instant),
    /// A preemptive update that has been finalized. We can now
    /// Send the replies to the clients and permanently apply the update
    /// to our state
    PreemptiveUpdateFinalized(SeqNo),
    /// Similarly to the [PreemptiveUpdateFinalized(_)] branch
    /// But with the added action of also taking a snapshot of the app state
    /// (After the update has been performed) and
    PreemptiveUpdateFinalizedAndGetAppstate(SeqNo),
    /// Execute an unordered update on the confirmed state (this will not
    /// take into account any pending preemptive updates, only updates which
    /// have been effectivized)
    ExecuteUnordered(UnorderedUpdateBatch<O>),
}

pub struct PreemptiveExecutorHandle<RQ> {
    e_tx: ChannelSyncTx<PreemptiveExecutionRequest<RQ>>,
    request_rx: ChannelSyncRx<PreemptiveExecutionRequest<RQ>>,
}

impl<RQ> PreemptiveExecutorHandle<RQ> {
    /// Creates a new `PreemptiveExecutorHandle` with the given execution channel sender.
    pub fn new(
        e_tx: ChannelSyncTx<PreemptiveExecutionRequest<RQ>>,
        request_rx: ChannelSyncRx<PreemptiveExecutionRequest<RQ>>,
    ) -> Self {
        Self { e_tx, request_rx }
    }

    /// Returns the execution request receiver channel.
    pub fn get_request_receiver(&self) -> &ChannelSyncRx<PreemptiveExecutionRequest<RQ>> {
        &self.request_rx
    }
}

impl<RQ> TExecutionHandle<RQ> for PreemptiveExecutorHandle<RQ>
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

impl<RQ> TDeterministicExecutionHandle<RQ> for PreemptiveExecutorHandle<RQ>
where
    RQ: Send,
{
    fn queue_update(&self, batch: UpdateBatch<RQ>) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::UpdateBatch(
                batch,
                Instant::now(),
            ))
            .context("Failed to send UpdateBatch request to executor")
    }

    fn queue_update_and_get_appstate(
        &self,
        batch: UpdateBatch<RQ>,
    ) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::UpdateBatchAndGetAppstate(
                batch,
                Instant::now(),
            ))
            .context("Failed to send UpdateFinalizedAndGetAppstateBatch request to executor")
    }
}

impl<RQ> TPreemptiveExecutionHandle<RQ> for PreemptiveExecutorHandle<RQ>
where
    RQ: Send,
{
    fn queue_preemptive_update(&self, batch: UpdateBatch<RQ>) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::PreemptiveUpdate(
                batch,
                Instant::now(),
            ))
            .context("Failed to send PreemptiveUpdate request to executor")
    }

    fn queue_update_finalized(&self, seq: SeqNo) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::PreemptiveUpdateFinalized(seq))
            .context("Failed to send UpdateFinalized request to executor")
    }

    fn queue_update_finalized_and_get_appstate(
        &self,
        seq: SeqNo,
    ) -> atlas_common::error::Result<()> {
        self.e_tx
            .send(PreemptiveExecutionRequest::PreemptiveUpdateFinalizedAndGetAppstate(seq))
            .context("Failed to send UpdateFinalizedAndGetAppstate request to executor")
    }
}

impl<RQ> Clone for PreemptiveExecutorHandle<RQ> {
    fn clone(&self) -> Self {
        Self {
            e_tx: self.e_tx.clone(),
            request_rx: self.request_rx.clone(),
        }
    }
}

#[cfg(test)]
mod specialization_guard {
    use super::PreemptiveExecutorHandle;
    use atlas_core::execution::TPreemptiveExecutorDecisionHandle;
    use atlas_smr_core::SMRRawReq;
    use atlas_smr_core::execution::SMRExecWrapper;
    use atlas_smr_core::execution::state_management::TPreemptiveExecutorStateHandle;

    /// Guards the wiring that makes preemptive execution actually happen.
    ///
    /// `atlas-smr-replica`'s decision log picks its preemptive code path via Rust
    /// specialization, bounded on these two traits. Specialization fails *silently*: if
    /// `SMRExecWrapper<PreemptiveExecutorHandle<_>>` stops satisfying them, the replica
    /// quietly falls back to the deterministic post-commit path and every executor in this
    /// crate stops executing speculatively -- while still building, running, and producing
    /// plausible-looking benchmark numbers.
    ///
    /// These assertions turn that silent regression into a compile error.
    fn _assert_preemptive_path_is_reachable<RQ: Send + 'static>() {
        fn requires_decision_handle<RQ, T: TPreemptiveExecutorDecisionHandle<SMRRawReq<RQ>>>() {}
        fn requires_state_handle<RQ, T: TPreemptiveExecutorStateHandle<SMRRawReq<RQ>>>() {}

        requires_decision_handle::<RQ, SMRExecWrapper<PreemptiveExecutorHandle<RQ>>>();
        requires_state_handle::<RQ, SMRExecWrapper<PreemptiveExecutorHandle<RQ>>>();
    }
}
