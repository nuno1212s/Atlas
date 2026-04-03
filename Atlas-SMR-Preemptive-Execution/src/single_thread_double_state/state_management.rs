use crate::single_thread_double_state::preemptive_worker::comm_handles::PreemptiveWorkMessage;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::UpdateBatch;
use atlas_smr_application::app::{Application, Request};
use getset::Getters;

/// Messages sent by the work distributor to the preemptive state management thread to trigger updates to the preemptive state.
pub(super) enum StateMessage<S> {
    ConfirmedStateReceived(SeqNo, S),
}

/// messages that the preemptive state management thread sends to the confirmed state management thread.
pub(super) enum PreemptiveToConfirmedMsg<R> {
    UpdateConfirmed(UpdateBatch<R>),
    RequestStateCopy(SeqNo),
}

/// Messages that the confirmed state management thread sends to the preemptive state management thread.
pub(super) struct ConfirmedToPreemptiveMsg<S>(pub SeqNo, pub S);
