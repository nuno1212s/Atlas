use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::UpdateBatch;

/// Messages sent by the work distributor to the preemptive state management thread to trigger updates to the preemptive state.
pub(super) enum StateMessage<S> {
    ConfirmedStateReceived(SeqNo, S),
}

/// messages that the preemptive state management thread sends to the confirmed state management thread.
#[allow(dead_code)]
pub(super) enum PreemptiveToConfirmedMsg<R> {
    UpdateConfirmed(UpdateBatch<R>),
    UpdateConfirmedEmitAppState(UpdateBatch<R>),
    RequestStateCopy(SeqNo),
}

/// Messages that the confirmed state management thread sends to the preemptive state management thread.
pub(super) struct ConfirmedToPreemptiveMsg<S>(#[allow(dead_code)] pub SeqNo, pub S);
