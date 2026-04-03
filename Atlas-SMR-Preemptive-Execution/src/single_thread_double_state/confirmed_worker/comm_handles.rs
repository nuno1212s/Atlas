use crate::single_thread_double_state::comm_handles::ConfirmedWorkerSharedChannels;
use crate::single_thread_double_state::state_management::{
    ConfirmedToPreemptiveMsg, PreemptiveToConfirmedMsg, StateMessage,
};
use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::maybe_vec::MaybeVec;
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use getset::Getters;
use std::time::Instant;

const CONFIRMED_WORKER_SIZE: usize = 256;

pub(super) fn initialize_handles<R, S>(
    shared_channels: ConfirmedWorkerSharedChannels<R, S>,
) -> (ConfirmedWorkerHandle<R, S>, ConfirmedChannels<R, S>) {
    let (update_message_tx, update_message_rx) = channel::sync::new_bounded_sync(
        CONFIRMED_WORKER_SIZE,
        Some("Confirmed worker update channel"),
    );

    let (state_message_tx, state_message_rx) = channel::sync::new_bounded_sync(
        CONFIRMED_WORKER_SIZE,
        Some("Confirmed worker state message channel"),
    );

    let confirmed_worker_handle = ConfirmedWorkerHandle::new(update_message_tx, state_message_tx);

    let (confirmed_to_preemptive_tx, preemptive_to_confirmed_rx) = shared_channels.into();

    let confirmed_worker_channels = ConfirmedChannels::new(
        preemptive_to_confirmed_rx,
        confirmed_to_preemptive_tx,
        update_message_rx,
        state_message_rx,
    );

    (confirmed_worker_handle, confirmed_worker_channels)
}

/// The handle for the outer layer to communicate with the confirmed worker.
#[derive(Getters)]
pub struct ConfirmedWorkerHandle<R, S> {
    #[get = "pub"]
    update_messages: ChannelSyncTx<ConfirmedUpdateMessage<R>>,
    #[get = "pub"]
    state_message_tx: ChannelSyncTx<StateMessage<S>>,
}

impl<R, S> ConfirmedWorkerHandle<R, S> {
    pub fn new(
        update_messages: ChannelSyncTx<ConfirmedUpdateMessage<R>>,
        state_message_tx: ChannelSyncTx<StateMessage<S>>,
    ) -> Self {
        Self {
            update_messages,
            state_message_tx,
        }
    }
}

impl<R, S> Clone for ConfirmedWorkerHandle<R, S> {
    fn clone(&self) -> Self {
        Self {
            update_messages: self.update_messages.clone(),
            state_message_tx: self.state_message_tx.clone(),
        }
    }
}

#[derive(Getters)]
pub(super) struct ConfirmedChannels<R, S> {
    #[get = "pub"]
    incoming_preemptive_msg: ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
    #[get = "pub"]
    outgoing_msg: ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
    #[get = "pub"]
    update_messages: ChannelSyncRx<ConfirmedUpdateMessage<R>>,
    #[get = "pub"]
    state_messages: ChannelSyncRx<StateMessage<S>>,
}

impl<R, S> ConfirmedChannels<R, S> {
    pub fn new(
        incoming_preemptive_msg: ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
        preemptive_worker_rx: ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
        update_messages: ChannelSyncRx<ConfirmedUpdateMessage<R>>,
        state_messages: ChannelSyncRx<StateMessage<S>>,
    ) -> Self {
        Self {
            incoming_preemptive_msg,
            outgoing_msg: preemptive_worker_rx,
            update_messages,
            state_messages,
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
    /// Notify the executor that there we have received a state transfer
    /// and we should poll the state channel to get a consistent state
    StateTransferAvailable,
}
