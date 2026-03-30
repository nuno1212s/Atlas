use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::UpdateBatch;
use atlas_smr_application::app::{Application, Request};
use getset::Getters;
use crate::single_thread_double_state::preemptive_worker::comm_handles::PreemptiveWorkMessage;

/// Messages sent by the work distributor to the preemptive state management thread to trigger updates to the preemptive state.
pub(super) enum PreemptiveStateMessage<S>
{
    ConfirmedStateReceived(SeqNo, S)
}

/// messages that the preemptive state management thread sends to the confirmed state management thread.
pub(super) enum PreemptiveToConfirmedMsg<R>
{
    UpdateConfirmed(UpdateBatch<R>),
    RequestStateCopy(SeqNo),
}

/// Messages that the confirmed state management thread sends to the preemptive state management thread.
pub(super) struct ConfirmedToPreemptiveMsg<S>(pub SeqNo, pub S);

#[derive(Getters)]
pub struct PreemptiveStateManagementHandle<R, S>
{
    #[get = "pub"]
    preemptive_state_handle: ChannelSyncTx<PreemptiveStateMessage<S>>,
    #[get = "pub"]
    preemptive_exec_handle: ChannelSyncTx<PreemptiveWorkMessage<R>>,
    #[get = "pub"]
    confirmed_updates_rx: ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
    #[get = "pub"]
    confirmed_states_tx: ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
}

impl<R, S> PreemptiveStateManagementHandle<R, S>
{
    pub fn new(
        preemptive_state_handle: ChannelSyncTx<PreemptiveStateMessage<S>>,
        preemptive_exec_handle: ChannelSyncTx<PreemptiveWorkMessage<R>>,
        confirmed_updates_rx: ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
        confirmed_states_tx: ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
    ) -> Self {
        Self {
            preemptive_state_handle,
            preemptive_exec_handle,
            confirmed_updates_rx,
            confirmed_states_tx,
        }
    }
}
