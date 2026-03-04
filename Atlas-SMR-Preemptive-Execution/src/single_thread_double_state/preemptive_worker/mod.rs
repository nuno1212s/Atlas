use crate::single_thread_double_state::preemptive_worker::comm_handles::PreemptiveChannels;
use crate::single_thread_double_state::preemptive_worker::preemptive_requests::PreemptiveRequestPipeline;
use crate::single_thread_double_state::state_management::{
    PreemptiveStateManagementHandle, PreemptiveStateMessage, PreemptiveToConfirmedMsg,
};
use atlas_common::channel::sync;
use atlas_common::ordering::SeqNo;
use atlas_common::{quiet_unwrap, unwrap_channel};
use atlas_smr_application::app::Application;
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::repliers::ExecutorReplier;
use std::sync::Arc;

pub(super) mod preemptive_requests;

mod comm_handles;

const PREEMPTIVE_WORKER_CHANNEL_SIZE: usize = 100;

pub fn initialize_preemptive_execution<A, S, NT, T>(
    state_seq: SeqNo,
    state: S,
    application: A,
    node: Arc<NT>,
) -> PreemptiveStateManagementHandle<A, S>
where
    A: Application<S> + Send + 'static,
    S: Send + 'static,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    T: ExecutorReplier,
{
    let (work_tx, work_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker Work Channel"),
    );
    let (confirmed_worker_tx, confirmed_worker_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker Confirmed Worker Channel"),
    );
    let (preemptive_worker_tx, preemptive_worker_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker Confirmed Worker Channel"),
    );

    let preemptive_channels =
        PreemptiveChannels::new(work_rx, confirmed_worker_tx, preemptive_worker_rx);

    let request_pipeline =
        PreemptiveRequestPipeline::new(preemptive_channels.clone(), (state_seq, state));

    spawn_preemptive_worker::<_, _, _, T>(preemptive_channels, request_pipeline, application, node);

    PreemptiveStateManagementHandle::new(work_tx, confirmed_worker_rx, preemptive_worker_tx)
}

fn spawn_preemptive_worker<A, S, NT, T>(
    channels: PreemptiveChannels<A, S>,
    request_pipeline: PreemptiveRequestPipeline<S, A>,
    application: A,
    node: Arc<NT>,
) where
    A: Application<S> + Send + 'static,
    S: Send + 'static,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    T: ExecutorReplier,
{
    std::thread::Builder::new()
        .name("Preemptive Worker Thread".to_string())
        .spawn(move || {
            preemptive_worker_loop::<_, _, _, T>(channels, request_pipeline, &application, node);
        })
        .expect("Failed to spawn preemptive worker thread");
}

fn preemptive_worker_loop<A, S, NT, T>(
    channels: PreemptiveChannels<A, S>,
    mut request_pipeline: PreemptiveRequestPipeline<S, A>,
    application: &A,
    node: Arc<NT>,
) where
    A: Application<S> + Send + 'static,
    S: Send + 'static,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    T: ExecutorReplier,
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
                            let (update_batch, replies) = request_pipeline.handle_update_confirmed(seq_no).into_inner();

                            T::execution_finished::<A::AppData, NT>(node.clone(), Some(seq_no), replies);

                            quiet_unwrap!(channels.confirmed_worker_tx().send(PreemptiveToConfirmedMsg::UpdateConfirmed(update_batch)));
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
