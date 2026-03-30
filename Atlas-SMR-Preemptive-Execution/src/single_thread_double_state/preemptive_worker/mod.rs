use crate::single_thread_double_state::RunMode;
use crate::single_thread_double_state::preemptive_worker::comm_handles::{
    PreemptiveChannels, PreemptiveWorkMessage,
};
use crate::single_thread_double_state::preemptive_worker::preemptive_requests::PreemptiveRequestPipeline;
use crate::single_thread_double_state::state_management::{
    PreemptiveStateManagementHandle, PreemptiveStateMessage, PreemptiveToConfirmedMsg,
};
use atlas_common::channel::{sync, RecvError, SendError};
use atlas_common::ordering::SeqNo;
use atlas_common::{quiet_unwrap, unwrap_channel};
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
    preemptive_channels: PreemptiveChannels<Request<A, S>, S>,
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
                RunMode::StateTransfer => self.preemptive_worker_state_transfer_mode()
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
                    PreemptiveWorkMessage::PollStateChannel => todo!()
                }

                Ok(())
            }
        }
    }

    fn preemptive_worker_state_transfer_mode(&mut self) -> Result<(), ChannelError> {
        sync::sync_select! {
            recv(unwrap_channel!(self.preemptive_channels.state_rx())) -> msg => {
                match msg.map_err(RecvError::from)? {
                    PreemptiveStateMessage::ConfirmedStateReceived(seq_no, state) => {
                        self.state.install_confirmed_state(state, seq_no);
                    }
                }

                Ok(())
            },
            recv(unwrap_channel!(self.preemptive_channels.confirmed_worker_rx())) -> msg => {
                let message = msg.map_err(RecvError::from)?;

                todo!()
            },
        }
    }
}

pub fn initialize_preemptive_execution<A, S, NT, T>(
    state_seq: SeqNo,
    state: S,
    application: Arc<A>,
    node: Arc<NT>,
) -> PreemptiveStateManagementHandle<Request<A, S>, S>
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

    let (confirmed_worker_tx, confirmed_worker_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker Confirmed Worker Channel"),
    );
    let (preemptive_worker_tx, preemptive_worker_rx) = sync::new_bounded_sync(
        PREEMPTIVE_WORKER_CHANNEL_SIZE,
        Some("Preemptive Worker Confirmed Worker Channel"),
    );

    let preemptive_channels =
        PreemptiveChannels::new(state_rx, work_rx, confirmed_worker_tx, preemptive_worker_rx);

    let request_pipeline =
        PreemptiveRequestPipeline::new(preemptive_channels.clone(), (state_seq, state));

    let preemptive_worker = PreemptiveWorker {
        state: request_pipeline,
        preemptive_channels,
        application,
        node,
        run_mode: RunMode::Normal,
    };

    preemptive_worker.spawn_and_execute::<T>();

    PreemptiveStateManagementHandle::new(
        state_tx,
        work_tx,
        confirmed_worker_rx,
        preemptive_worker_tx,
    )
}

#[derive(Debug, Error)]
enum ChannelError {
    #[error("Receive error: {0}")]
    RecvError(#[from] RecvError),
    #[error("Send error: {0}")]
    SendError(#[from] SendError)
}