use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::UpdateBatch;
use atlas_smr_application::app::{Application, Request};

/// Messages sent by the work distributor to the preemptive state management thread to trigger updates to the preemptive state.
pub(super) enum PreemptiveStateMessage<A, S>
where
    A: Application<S>,
{
    ConfirmedStateReceived(SeqNo, S),
    PreemptiveUpdate(UpdateBatch<Request<A, S>>),
    ConfirmedUpdate(SeqNo),
}

/// messages that the preemptive state management thread sends to the confirmed state management thread.
pub(super) enum PreemptiveToConfirmedMsg<A, S>
where
    A: Application<S>,
{
    UpdateConfirmed(UpdateBatch<Request<A, S>>),
    RequestStateCopy(SeqNo),
}

/// Messages that the confirmed state management thread sends to the preemptive state management thread.
pub(super) struct ConfirmedToPreemptiveMsg<S>(pub SeqNo, pub S);

pub struct PreemptiveStateManagementHandle<A, S>
where
    A: Application<S>,
{
    preemptive_execution_handle: ChannelSyncTx<PreemptiveStateMessage<A, S>>,
}
