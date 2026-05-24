use crate::metric::DS_BACKTRACK_COUNT_ID;
use crate::single_thread_double_state::RunMode;
use crate::single_thread_double_state::comm_handles::PreemptiveWorkerSharedChannels;
use crate::single_thread_double_state::preemptive_worker::comm_handles::{
    PreemptiveWorkMessage, PreemptiveWorkerChannels, PreemptiveWorkerHandle,
    RequestLatestStateError,
};
use crate::single_thread_double_state::preemptive_worker::preemptive_requests::{
    BacktrackError, ConfirmedUpdateError, ExecuteUpdateError, HandleUpdateConfirmedError,
    PreemptiveState,
};
use crate::single_thread_double_state::state_management::StateMessage;
use atlas_common::channel::{NoRetChannelErr, RecvError, sync};
use atlas_common::ordering::SeqNo;
use atlas_common::unwrap_channel;
use atlas_core::execution::requests::UpdateBatch;
use atlas_metrics::metrics::metric_increment;
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
    state: PreemptiveState<S, A>,
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
                error!("Preemptive worker thread failed with error: {err}");

                break;
            }
        }
    }

    fn preemptive_worker_normal_mode<T>(&mut self) -> Result<(), PreemptiveWorkerError>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier,
    {
        sync::sync_select! {
            recv(unwrap_channel!(self.preemptive_channels.work_rx())) -> msg => {
                match msg.map_err(|e| NoRetChannelErr::from(RecvError::from(e)))? {
                    PreemptiveWorkMessage::PreemptiveUpdate(update_batch) => {
                        self.handle_preemptive_update::<T>(update_batch)?;
                    }
                    PreemptiveWorkMessage::PreemptiveUpdateConfirmed(seq_no) => {
                        let update_batch = self.handle_preemptive_update_confirmed::<T>(seq_no)?;

                        self.preemptive_channels.send_update_confirmed(update_batch);
                    },
                    PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(seq_no) => {
                        let update_batch = self.handle_preemptive_update_confirmed::<T>(seq_no)?;

                        self.preemptive_channels.send_update_confirmed_get_appstate(update_batch);
                    }
                    PreemptiveWorkMessage::ConfirmedUpdate(update_batch) => {
                        self.handle_confirmed_update::<T>(update_batch, false)?;
                    }
                    PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(update_batch) => {
                        self.handle_confirmed_update::<T>(update_batch, true)?;
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

    fn handle_preemptive_update_confirmed<T>(
        &mut self,
        update_seq: SeqNo,
    ) -> Result<UpdateBatch<Request<A, S>>, PreemptiveWorkerError>
    where
        T: ExecutorReplier,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        let (update_batch, replies) = self.state.handle_update_confirmed(update_seq)?.into_inner();

        T::execution_finished::<A::AppData, NT>(self.node.clone(), Some(update_seq), replies);

        Ok(update_batch)
    }

    fn handle_preemptive_update<T>(
        &mut self,
        update: UpdateBatch<Request<A, S>>,
    ) -> Result<(), PreemptiveWorkerError>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier,
    {
        if let Err(err) = self
            .state
            .handle_preemptive_update(&self.application, update)
        {
            match err {
                ExecuteUpdateError::Backtracking(_, batch) => {
                    self.handle_backtracking_request::<T>(batch)?;
                }
                ExecuteUpdateError::FutureRequest(seq, current) => {
                    error!(
                        "Preemptive update at seq {seq:?} is ahead of current head {current:?}; dropping"
                    );
                }
                ExecuteUpdateError::ChannelErr(e) => {
                    return Err(PreemptiveWorkerError::Channel(e));
                }
            }
        }

        Ok(())
    }

    fn preemptive_worker_state_transfer_mode(&mut self) -> Result<(), PreemptiveWorkerError> {
        sync::sync_select! {
            recv(unwrap_channel!(self.preemptive_channels.state_rx())) -> msg => {
                match msg.map_err(|e| NoRetChannelErr::from(RecvError::from(e)))? {
                    StateMessage::ConfirmedStateReceived(seq_no, state) => {
                        self.state.install_confirmed_state(seq_no, state);
                    }
                }

                self.set_run_mode(RunMode::Normal);

                Ok(())
            },
            recv(unwrap_channel!(self.preemptive_channels.confirmed_worker_rx())) -> msg => {
                let _message = msg.map_err(|e| NoRetChannelErr::from(RecvError::from(e)))?;

                todo!()
            },
        }
    }

    fn handle_confirmed_update<T>(
        &mut self,
        update_batch: UpdateBatch<Request<A, S>>,
        get_appstate: bool,
    ) -> Result<(), PreemptiveWorkerError>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier,
    {
        let seq = update_batch.seq_no();
        let batch_for_confirmed = update_batch.clone();

        // Forward to the confirmed worker first so it can start executing
        // on its authoritative state in parallel with our local execution below.
        if get_appstate {
            self.preemptive_channels
                .send_update_confirmed_get_appstate(batch_for_confirmed);
        } else {
            self.preemptive_channels
                .send_update_confirmed(batch_for_confirmed);
        }

        let replies = self
            .state
            .handle_confirmed_update(&self.application, update_batch)?;

        T::execution_finished::<A::AppData, NT>(self.node.clone(), Some(seq), replies);

        Ok(())
    }

    fn handle_backtracking_request<T>(
        &mut self,
        update_batch: UpdateBatch<Request<A, S>>,
    ) -> Result<(), PreemptiveWorkerError>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier,
    {
        metric_increment(DS_BACKTRACK_COUNT_ID, Some(1));
        let state = self.preemptive_channels.request_latest_confirmed_state()?;

        self.state
            .backtrack(&self.application, state.0, state.1, update_batch.seq_no())?;

        self.handle_preemptive_update::<T>(update_batch)?;

        Ok(())
    }

    fn set_run_mode(&mut self, run_mode: RunMode) {
        self.run_mode = run_mode;
    }
}

pub fn initialize_preemptive_execution<A, S, NT, T>(
    (state_seq, state): (SeqNo, S),
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

    let request_pipeline = PreemptiveState::new((state_seq, state));

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
enum PreemptiveWorkerError {
    #[error("Channel error: {0}")]
    Channel(#[from] NoRetChannelErr),
    #[error("Confirmed update invariant violated: {0}")]
    ConfirmedUpdate(#[from] ConfirmedUpdateError),
    #[error("Failed to confirm preemptive update: {0}")]
    HandleUpdateConfirmed(#[from] HandleUpdateConfirmedError),
    #[error("Backtrack operation failed: {0}")]
    Backtrack(#[from] BacktrackError),
    #[error("Failed to request latest confirmed state: {0}")]
    RequestLatestState(#[from] RequestLatestStateError),
}
