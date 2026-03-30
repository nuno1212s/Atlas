use crate::single_thread_double_state::state_management::{
    ConfirmedToPreemptiveMsg, PreemptiveToConfirmedMsg,
};
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::maybe_vec::MaybeVec;
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use getset::Getters;
use std::time::Instant;

#[derive(Getters)]
pub(super) struct ConfirmedChannels<R, S> {
    #[get = "pub"]
    incoming_msg_preemptive: ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
    #[get = "pub"]
    outgoing_msg: ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
    #[get = "pub"]
    update_messages: ChannelSyncRx<ConfirmedUpdateMessage<R>>,
}

impl<R, S> ConfirmedChannels<R, S> {
    pub fn new(
        preemptive_worker_tx: ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
        preemptive_worker_rx: ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
        update_messages: ChannelSyncRx<ConfirmedUpdateMessage<R>>,
    ) -> Self {
        Self {
            incoming_msg_preemptive: preemptive_worker_tx,
            outgoing_msg: preemptive_worker_rx,
            update_messages,
        }
    }
}

pub enum ConfirmedUpdateMessage<R> {
    /// Instruct the executor to catch up to a quorum of updates.
    CatchUp(MaybeVec<UpdateBatch<R>>),
    /// A decided, finalized update batch to be executed.
    /// The Instant represents the time at which the update was originally queued for execution.
    UpdateBatch(UpdateBatch<R>, Instant),
    /// A decided, finalized update batch to be executed, and the application state to be retrieved after execution.
    /// The Instant represents the time at which the update was originally queued for execution.
    UpdateFinalizedAndGetAppstateBatch(UpdateBatch<R>, Instant),
    /// Execute an unordered update on the confirmed state (this will not
    /// take into account any pending preemptive updates, only updates which
    /// have been effectivized)
    ExecuteUnordered(UnorderedUpdateBatch<R>),
}
