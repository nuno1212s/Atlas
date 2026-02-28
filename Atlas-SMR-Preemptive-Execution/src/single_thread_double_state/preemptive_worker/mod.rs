use crate::single_thread_double_state::preemptive_worker::comm_handles::PreemptiveChannels;
use crate::single_thread_double_state::preemptive_worker::preemptive_requests::PreemptiveRequestPipeline;
use crate::single_thread_double_state::state_management::{
    PreemptiveStateManagementHandle, PreemptiveStateMessage,
};
use atlas_common::channel::sync;
use atlas_common::ordering::SeqNo;
use atlas_common::unwrap_channel;
use atlas_smr_application::app::Application;

pub(super) mod preemptive_requests;

mod comm_handles;

const PREEMPTIVE_WORKER_CHANNEL_SIZE: usize = 100;

pub fn initialize_preemptive_execution<A, S>(
    state_seq: SeqNo,
    state: S,
    application: A
) -> PreemptiveStateManagementHandle<A, S>
where
    A: Application<S> + Send + 'static,
    S: Send + 'static,
{
    let (work_tx, work_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker Work Channel"),
    );
    let (confirmed_worker_tx, confirmed_worker_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker Confirmed Worker Channel"),
    );
    let (preemptive_worker_tx, preemptive_worker_rx) =
        sync::new_bounded_sync(
            PREEMPTIVE_WORKER_CHANNEL_SIZE,
            Some("Preemptive Worker Confirmed Worker Channel"),
        );

    let preemptive_channels =
        PreemptiveChannels::new(work_rx, confirmed_worker_tx, preemptive_worker_rx);

    let request_pipeline =
        PreemptiveRequestPipeline::new(preemptive_channels.clone(), (state_seq, state));

    spawn_preemptive_worker(preemptive_channels, request_pipeline, application);
    
    PreemptiveStateManagementHandle::new(work_tx, confirmed_worker_rx, preemptive_worker_tx)
}

fn spawn_preemptive_worker<A, S>(
    channels: PreemptiveChannels<A, S>,
    request_pipeline: PreemptiveRequestPipeline<S, A>,
    application: A
) where
    A: Application<S> + Send + 'static,
    S: Send + 'static,
{
    std::thread::spawn(move || {
        preemptive_worker_loop(channels, request_pipeline, &application);
    });
}

fn preemptive_worker_loop<A, S>(
    channels: PreemptiveChannels<A, S>,
    mut request_pipeline: PreemptiveRequestPipeline<S, A>,
    application: &A,
) where
    A: Application<S> + Send + 'static,
    S: Send + 'static,
{
    loop {
        sync::sync_select! {
            recv(unwrap_channel!(channels.work_rx())) -> msg => {
                if let Ok(message) = msg {
                    match message {
                        PreemptiveStateMessage::ConfirmedStateReceived(seq_no, state) => {
                            request_pipeline.install_confirmed_state(state, seq_no);
                        }
                        PreemptiveStateMessage::PreemptiveUpdate(update_batch) => {
                            request_pipeline.handle_preemptive_update(application, update_batch);
                        }
                        PreemptiveStateMessage::ConfirmedUpdate(seq_no) => {
                            request_pipeline.handle_update_confirmed(seq_no);
                        }
                    }
                } else {
                    //TODO Handle error receiving message from work channel
                }
            },
            recv(unwrap_channel!(channels.confirmed_worker_rx())) -> msg => {
                match msg {
                    Ok(message) => {
                        //TODO Handle messages from the confirmed state management thread
                    }
                    Err(err) => {
                        //TODO Handle error receiving message from confirmed state management thread
                    }
                }
            },
        }
    }
}
