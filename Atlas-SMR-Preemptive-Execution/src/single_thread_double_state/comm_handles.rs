use crate::single_thread_double_state::state_management::{
    ConfirmedToPreemptiveMsg, PreemptiveToConfirmedMsg,
};
use crate::single_thread_double_state::{EXECUTING_BUFFER, STATE_BUFFER};
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use getset::Getters;

/// The channels for the Confirmed Worker to use in communication with the Preemptive Worker.
#[derive(Getters)]
pub struct ConfirmedWorkerSharedChannels<R, S> {
    #[get = "pub"]
    confirmed_to_preemptive_tx: ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
    #[get = "pub"]
    preemptive_to_confirmed_rx: ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
}

impl<R, S> From<ConfirmedWorkerSharedChannels<R, S>>
    for (
        ChannelSyncTx<ConfirmedToPreemptiveMsg<S>>,
        ChannelSyncRx<PreemptiveToConfirmedMsg<R>>,
    )
{
    fn from(value: ConfirmedWorkerSharedChannels<R, S>) -> Self {
        (
            value.confirmed_to_preemptive_tx,
            value.preemptive_to_confirmed_rx,
        )
    }
}

/// The channels for the Preemptive Worker to use in communication with the Confirmed Worker.
pub struct PreemptiveWorkerSharedChannels<R, S> {
    confirmed_to_preemptive_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,

    preemptive_to_confirmed_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<R>>,
}

impl<R, S> From<PreemptiveWorkerSharedChannels<R, S>>
    for (
        ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
        ChannelSyncTx<PreemptiveToConfirmedMsg<R>>,
    )
{
    fn from(value: PreemptiveWorkerSharedChannels<R, S>) -> Self {
        (
            value.confirmed_to_preemptive_rx,
            value.preemptive_to_confirmed_tx,
        )
    }
}

pub fn initialize_shared_channels<R, S>() -> (
    ConfirmedWorkerSharedChannels<R, S>,
    PreemptiveWorkerSharedChannels<R, S>,
) {
    let (confirmed_to_preemptive_tx, confirmed_to_preemptive_rx) =
        atlas_common::channel::sync::new_bounded_sync(
            STATE_BUFFER,
            Some("Confirmed to Preemptive Channel"),
        );

    let (preemptive_to_confirmed_tx, preemptive_to_confirmed_rx) =
        atlas_common::channel::sync::new_bounded_sync(
            EXECUTING_BUFFER,
            Some("Preemptive to Confirmed Channel"),
        );

    (
        ConfirmedWorkerSharedChannels {
            confirmed_to_preemptive_tx,
            preemptive_to_confirmed_rx,
        },
        PreemptiveWorkerSharedChannels {
            confirmed_to_preemptive_rx,
            preemptive_to_confirmed_tx,
        },
    )
}
