use crate::single_thread_double_state::RunMode;
use crate::single_thread_double_state::comm_handles::PreemptiveWorkerSharedChannels;
use crate::single_thread_double_state::preemptive_worker::comm_handles::{
    PreemptiveWorkMessage, PreemptiveWorkerChannels, PreemptiveWorkerHandle,
};
use crate::single_thread_double_state::preemptive_worker::preemptive_requests::PreemptiveRequestPipeline;
use crate::single_thread_double_state::state_management::{PreemptiveToConfirmedMsg, StateMessage};
use atlas_common::channel::{RecvError, SendError, sync};
use atlas_common::ordering::SeqNo;
use atlas_common::unwrap_channel;
use atlas_smr_application::app::{Application, Request};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::repliers::ExecutorReplier;
use std::sync::Arc;
use thiserror::Error;
use tracing::error;

pub(super) mod preemptive_requests;

pub(super) mod comm_handles;

const PREEMPTIVE_WORKER_CHANNEL_SIZE: usize = 100;

struct PreemptiveWorker<A, S, NT>
where
    A: Application<S>,
{
    state: PreemptiveRequestPipeline<S, A>,
    preemptive_channels: PreemptiveWorkerChannels<Request<A, S>, S>,
    application: Arc<A>,
    node: Arc<NT>,
    run_mode: RunMode,
}

impl<A, S, NT> PreemptiveWorker<A, S, NT>
where
    A: Application<S>,
{
    fn spawn_and_execute<T>(mut self)
    where
        A: 'static,
        S: Send + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier,
    {
        std::thread::Builder::new()
            .name("Preemptive Worker Thread".to_string())
            .spawn(move || {
                self.preemptive_worker_loop::<T>();
            })
            .expect("Failed to spawn preemptive worker thread");
    }

    fn preemptive_worker_loop<T>(&mut self)
    where
        S: Send + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier,
    {
        loop {
            let result = match &self.run_mode {
                RunMode::Normal => self.preemptive_worker_normal_mode::<T>(),
                RunMode::StateTransfer => self.preemptive_worker_state_transfer_mode(),
            };

            if let Err(err) = result {
                error!("Preemptive worker thread failed with error: {:?}", err);

                break;
            }
        }
    }

    fn preemptive_worker_normal_mode<T>(&mut self) -> Result<(), ChannelError>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier,
    {
        sync::sync_select! {
            recv(unwrap_channel!(self.preemptive_channels.work_rx())) -> msg => {
                match msg.map_err(RecvError::from)? {
                    PreemptiveWorkMessage::PreemptiveUpdate(update_batch) => {
                        self.state.handle_preemptive_update(&self.application, update_batch);
                    }
                    PreemptiveWorkMessage::ConfirmedUpdate(seq_no) => {
                        let (update_batch, replies) = self.state.handle_update_confirmed(seq_no).into_inner();

                        T::execution_finished::<A::AppData, NT>(self.node.clone(), Some(seq_no), replies);

                        self.preemptive_channels.confirmed_worker_tx().send(PreemptiveToConfirmedMsg::UpdateConfirmed(update_batch))?;
                    },
                    PreemptiveWorkMessage::ConfirmedUpdateEmitAppState(seq_no) => {
                        let (update_batch, replies) = self.state.handle_update_confirmed(seq_no).into_inner();

                        T::execution_finished::<A::AppData, NT>(self.node.clone(), Some(seq_no), replies);

                        self.preemptive_channels.confirmed_worker_tx().send(PreemptiveToConfirmedMsg::UpdateConfirmedEmitAppState(update_batch))?;
                    }
                    PreemptiveWorkMessage::CatchUp(confirmed_batches) => {
                        self.state.handle_catch_up(&self.application, confirmed_batches);
                    }
                    PreemptiveWorkMessage::PollStateChannel => {
                        self.set_run_mode(RunMode::StateTransfer);
                    }
                }

                Ok(())
            }
        }
    }

    fn preemptive_worker_state_transfer_mode(&mut self) -> Result<(), ChannelError> {
        sync::sync_select! {
            recv(unwrap_channel!(self.preemptive_channels.state_rx())) -> msg => {
                match msg.map_err(RecvError::from)? {
                    StateMessage::ConfirmedStateReceived(seq_no, state) => {
                        self.state.install_confirmed_state(state, seq_no);
                    }
                }

                self.set_run_mode(RunMode::Normal);

                Ok(())
            },
            recv(unwrap_channel!(self.preemptive_channels.confirmed_worker_rx())) -> msg => {
                let _message = msg.map_err(RecvError::from)?;

                todo!()
            },
        }
    }

    fn set_run_mode(&mut self, run_mode: RunMode) {
        self.run_mode = run_mode;
    }
}

pub fn initialize_preemptive_execution<A, S, NT, T>(
    state_seq: SeqNo,
    state: S,
    application: Arc<A>,
    node: Arc<NT>,
    shared_channels: PreemptiveWorkerSharedChannels<Request<A, S>, S>,
) -> PreemptiveWorkerHandle<Request<A, S>, S>
where
    A: Application<S> + Send + 'static,
    S: Send + 'static,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    T: ExecutorReplier,
{
    let (state_tx, state_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker State Channel"),
    );

    let (work_tx, work_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker Work Channel"),
    );

    let (confirmed_to_preemptive_rx, preemptive_to_confirmed_tx) = shared_channels.into();

    let preemptive_channels = PreemptiveWorkerChannels::new(
        state_rx,
        work_rx,
        preemptive_to_confirmed_tx,
        confirmed_to_preemptive_rx,
    );

    let request_pipeline = PreemptiveRequestPipeline::new((state_seq, state));

    let preemptive_worker = PreemptiveWorker {
        state: request_pipeline,
        preemptive_channels,
        application,
        node,
        run_mode: RunMode::Normal,
    };

    preemptive_worker.spawn_and_execute::<T>();

    PreemptiveWorkerHandle::new(state_tx, work_tx)
}

#[derive(Debug, Error)]
enum ChannelError {
    #[error("Receive error: {0}")]
    RecvError(#[from] RecvError),
    #[error("Send error: {0}")]
    SendError(#[from] SendError),
}
