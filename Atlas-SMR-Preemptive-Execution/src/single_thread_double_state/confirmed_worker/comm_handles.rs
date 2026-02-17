use crate::single_thread_double_state::state_management::{
    ConfirmedToPreemptiveMsg, PreemptiveToConfirmedMsg,
};
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_smr_application::app::Application;

pub(super) struct ConfirmedChannels<A, S>
where
    A: Application<S>,
{
    preemptive_worker_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<A, S>>,
    preemptive_worker_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
}

impl<A, S> ConfirmedChannels<A, S>
where
    A: Application<S>,
{
    pub fn new(
        preemptive_worker_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<A, S>>,
        preemptive_worker_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
    ) -> Self {
        Self {
            preemptive_worker_tx,
            preemptive_worker_rx,
        }
    }
}
