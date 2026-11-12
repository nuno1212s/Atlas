use crate::single_thread_double_state::comm_handles::ConfirmedWorkerSharedChannels;
use crate::single_thread_double_state::state_management::{
    ConfirmedToPreemptiveMsg, PreemptiveToConfirmedMsg, StateMessage,
};
use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use atlas_smr_application::state::monolithic_state::AppStateMessage;
use getset::Getters;

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

    let (baseline_ack_tx, baseline_ack_rx) = channel::sync::new_bounded_sync(
        CONFIRMED_WORKER_SIZE,
        Some("Confirmed worker baseline ack channel"),
    );

    let confirmed_worker_handle =
        ConfirmedWorkerHandle::new(update_message_tx, state_message_tx, baseline_ack_rx);

    let (confirmed_to_preemptive_tx, preemptive_to_confirmed_rx, state_emission_channel) =
        shared_channels.into();

    let confirmed_worker_channels = ConfirmedChannels::new(
        preemptive_to_confirmed_rx,
        confirmed_to_preemptive_tx,
        update_message_rx,
        state_message_rx,
        state_emission_channel,
        baseline_ack_tx,
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
    /// The confirmed worker's sequence number after each state install or catch-up, sent
    /// once it has finished. The orchestrator waits on this before handing the same state
    /// or batches to the preemptive worker. See `await_confirmed_baseline`.
    #[get = "pub"]
    baseline_ack_rx: ChannelSyncRx<SeqNo>,
}

impl<R, S> ConfirmedWorkerHandle<R, S> {
    pub fn new(
        update_messages: ChannelSyncTx<ConfirmedUpdateMessage<R>>,
        state_message_tx: ChannelSyncTx<StateMessage<S>>,
        baseline_ack_rx: ChannelSyncRx<SeqNo>,
    ) -> Self {
        Self {
            update_messages,
            state_message_tx,
            baseline_ack_rx,
        }
    }
}

impl<R, S> Clone for ConfirmedWorkerHandle<R, S> {
    fn clone(&self) -> Self {
        Self {
            update_messages: self.update_messages.clone(),
            state_message_tx: self.state_message_tx.clone(),
            baseline_ack_rx: self.baseline_ack_rx.clone(),
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
    #[get = "pub"]
    state_emission_channel: ChannelSyncTx<AppStateMessage<S>>,
    #[get = "pub"]
    baseline_ack: ChannelSyncTx<SeqNo>,
}

impl<R, S> ConfirmedChannels<R, S> {
    pub fn new(
        incoming_preemptive_msg: ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
        preemptive_worker_rx: ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
        update_messages: ChannelSyncRx<ConfirmedUpdateMessage<R>>,
        state_messages: ChannelSyncRx<StateMessage<S>>,
        state_emission_channel: ChannelSyncTx<AppStateMessage<S>>,
        baseline_ack: ChannelSyncTx<SeqNo>,
    ) -> Self {
        Self {
            incoming_preemptive_msg,
            outgoing_msg: preemptive_worker_rx,
            update_messages,
            state_messages,
            state_emission_channel,
            baseline_ack,
        }
    }
}

pub enum ConfirmedUpdateMessage<R> {
    /// Instruct the executor to catch up to a quorum of updates.
    /// The worker acknowledges on `baseline_ack` once it has applied them.
    CatchUp(MaybeVec<UpdateBatch<R>>),
    /// Execute an unordered update on the confirmed state (this will not
    /// take into account any pending preemptive updates, only updates which
    /// have been effectivized)
    ExecuteUnordered(UnorderedUpdateBatch<R>),
    /// Notify the executor that there we have received a state transfer
    /// and we should poll the state channel to get a consistent state.
    /// The worker acknowledges the install on `baseline_ack`.
    StateTransferAvailable,
}
