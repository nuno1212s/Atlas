use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::UpdateBatch;
use std::time::Instant;

/// Messages sent by the work distributor to the preemptive state management thread to trigger updates to the preemptive state.
pub(super) enum StateMessage<S> {
    ConfirmedStateReceived(SeqNo, S),
}

/// Messages that the preemptive state management thread sends to the confirmed state management thread.
#[allow(dead_code)]
pub(super) enum PreemptiveToConfirmedMsg<R> {
    /// A confirmed batch forwarded to the confirmed worker for authoritative execution.
    /// The Instant records when the message was sent so the confirmed worker can measure its
    /// processing latency.
    UpdateConfirmed(UpdateBatch<R>, Instant),
    UpdateConfirmedEmitAppState(UpdateBatch<R>, Instant),
    RequestStateCopy,
}

/// Messages that the confirmed state management thread sends to the preemptive state management thread.
pub(super) struct ConfirmedToPreemptiveMsg<S>(#[allow(dead_code)] pub SeqNo, pub S);
