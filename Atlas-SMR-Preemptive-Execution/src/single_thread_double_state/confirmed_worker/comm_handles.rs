use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_smr_application::app::Application;
use crate::single_thread_double_state::state_management::{ConfirmedStateMessage, ConfirmedToPreemptiveMsg, PreemptiveToConfirmedMsg};

pub(super) struct ConfirmedChannels<A, S>
where
    A: Application<S>,
{
    work_rx: ChannelSyncRx<ConfirmedStateMessage<A, S>>,
    preemptive_worker_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<A, S>>,
    preemptive_worker_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
}

impl<A, S> ConfirmedChannels<A, S>
where
    A: Application<S>,
{
    pub fn new(
        work_rx: ChannelSyncRx<ConfirmedStateMessage<A, S>>,
        preemptive_worker_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<A, S>>,
        preemptive_worker_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
    ) -> Self {
        Self {
            work_rx,
            preemptive_worker_tx,
            preemptive_worker_rx,
        }
    }
}

