use atlas_common::channel::sync::ChannelSyncTx;
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::{Update, UpdateBatch};
use atlas_smr_application::app::{Application, Request};

enum PreemptiveStateMessage<A, S>
where
    A: Application<S>,
{
    PreemptiveUpdate(UpdateBatch<Request<A, S>>),
    ConfirmedUpdate(SeqNo)
}

enum ConfirmedStateMessage<A, S>
where
    A: Application<S>,
{
    Update(UpdateBatch<Request<A, S>>),
}

enum ManagementMessage<A, S>
where A: Application<S> {
    UpdateConfirmed(UpdateBatch<Request<A, S>>),
}

struct PreemptiveStateManagementHandle<A, S>
where
    A: Application<S>,
{
    preemptive_execution_handle: ChannelSyncTx<PreemptiveStateMessage<A, S>>,
    confirmed_execution_handle: ChannelSyncTx<ConfirmedStateMessage<A, S>>,
}
