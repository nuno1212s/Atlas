use crate::exec_handle::{PreemptiveExecutionRequest, PreemptiveExecutorHandle};
use crate::single_thread_double_state::confirmed_worker::comm_handles::{
    ConfirmedUpdateMessage, ConfirmedWorkerHandle,
};
use crate::single_thread_double_state::preemptive_worker::comm_handles::{
    PreemptiveWorkMessage, PreemptiveWorkerHandle,
};
use crate::single_thread_double_state::state_management::StateMessage;
use atlas_common::channel::{
    self, NoRetChannelErr,
    sync::{ChannelSyncRx, ChannelSyncTx},
};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::{error, quiet_unwrap, unwrap_channel};
use atlas_smr_application::{
    app::{Application, Request},
    state::monolithic_state::{AppStateMessage, InstallStateMessage, MonolithicState},
};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::executors::monolithic_state::MonStateInstallHandle;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::repliers::ExecutorReplier;
use std::sync::Arc;
use std::time::Instant;

mod comm_handles;
mod confirmed_worker;
mod duplicate_state;
mod preemptive_worker;
mod state_management;

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;

enum RunMode {
    Normal,
    StateTransfer,
}

pub struct PreemptiveDuplicateStateMonolithicExecutor<S, A, NT>
where
    S: MonolithicState + 'static,
    A: Application<S> + 'static,
{
    run_mode: RunMode,

    confirmed_worker: ConfirmedWorkerHandle<Request<A, S>, S>,
    preemptive_worker: PreemptiveWorkerHandle<Request<A, S>, S>,

    work_rx: ChannelSyncRx<PreemptiveExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,

    send_node: Arc<NT>,
}

pub fn init_handle<A, S>() -> PreemptiveExecutorHandle<Request<A, S>>
where
    S: MonolithicState,
    A: Application<S>,
{
    let (tx, rx) = channel::sync::new_bounded_sync(
        EXECUTING_BUFFER,
        Some("ST Preemptive Duplicate State Executor Work Channel"),
    );

    PreemptiveExecutorHandle::new(tx, rx)
}

impl<S, A, NT> PreemptiveDuplicateStateMonolithicExecutor<S, A, NT>
where
    S: MonolithicState + 'static + Sync + Send,
    A: Application<S> + 'static,
{
    pub fn init<T>(
        handle: ChannelSyncRx<PreemptiveExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> error::Result<MonStateInstallHandle<S>>
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        let wrapped_application = Arc::new(service);

        let (seq, state) = if let Some((mut state, requests)) = initial_state {
            for request in requests {
                wrapped_application.update(&mut state, request.clone());
            }

            (SeqNo::ZERO, state)
        } else {
            (SeqNo::ZERO, A::initial_state()?)
        };

        let (state_tx, state_rx) = channel::sync::new_bounded_sync(
            STATE_BUFFER,
            Some("ST Monolithic Executor Work InstState"),
        );

        let (checkpoint_tx, checkpoint_rx) =
            channel::sync::new_bounded_sync(STATE_BUFFER, Some("ST Monolithic Executor AppState"));

        let (confirmed_worker_shared_channels, preemptive_worker_shared_channels) =
            comm_handles::initialize_shared_channels(checkpoint_tx);

        let confirmed_worker = confirmed_worker::init_confirmed_worker::<A, S, NT, T>(
            (seq, state.clone()),
            wrapped_application.clone(),
            send_node.clone(),
            confirmed_worker_shared_channels,
        );
        let preemptive_state_handle =
            preemptive_worker::initialize_preemptive_execution::<A, S, NT, T>(
                SeqNo::ZERO,
                state,
                wrapped_application.clone(),
                send_node.clone(),
                preemptive_worker_shared_channels,
            );

        let mut executor = Self {
            run_mode: RunMode::Normal,
            confirmed_worker,
            preemptive_worker: preemptive_state_handle,
            work_rx: handle,
            state_rx,
            send_node,
        };

        std::thread::Builder::new()
            .name("Executor thread".to_string())
            .spawn(move || executor.worker::<T>())
            .expect("Failed to start execution thread");

        Ok((state_tx, checkpoint_rx))
    }

    fn set_run_mode(&mut self, run_mode: RunMode) {
        self.run_mode = run_mode;
    }

    fn worker<T>(&mut self)
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        match &self.run_mode {
            RunMode::Normal => {
                channel::sync::sync_select! {
                     recv(unwrap_channel!(self.work_rx)) -> exec_req => {
                        if let Ok(exec_req) = exec_req {
                            self.handle_preemptive_execution_request(exec_req);
                        }
                    }
                }
            }
            RunMode::StateTransfer => {
                let message = quiet_unwrap!(self.state_rx.recv());

                let seq = message.sequence_number();
                let state = message.into_state();

                // when we receive a state message, we can immediately
                // Send it to both the confirmed worker and the preemptive worker.
                quiet_unwrap!(self.send_state_to_preemptive_worker(seq, state.clone()));
                quiet_unwrap!(self.send_state_to_confirmed_worker(seq, state));
            }
        }
    }

    fn send_state_to_confirmed_worker(
        &mut self,
        seq: SeqNo,
        state: S,
    ) -> Result<(), NoRetChannelErr> {
        self.confirmed_worker
            .update_messages()
            .send(ConfirmedUpdateMessage::StateTransferAvailable)?;
        self.confirmed_worker
            .state_message_tx()
            .send(StateMessage::ConfirmedStateReceived(seq, state.clone()))?;

        Ok(())
    }

    fn send_state_to_preemptive_worker(
        &mut self,
        seq: SeqNo,
        state: S,
    ) -> Result<(), NoRetChannelErr> {
        self.preemptive_worker
            .preemptive_exec_handle()
            .send(PreemptiveWorkMessage::PollStateChannel)?;
        self.preemptive_worker
            .preemptive_state_handle()
            .send(StateMessage::ConfirmedStateReceived(seq, state.clone()))?;

        Ok(())
    }

    fn handle_preemptive_execution_request(
        &mut self,
        execution_request: PreemptiveExecutionRequest<Request<A, S>>,
    ) where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        match execution_request {
            PreemptiveExecutionRequest::PollStateChannel => {
                self.set_run_mode(RunMode::StateTransfer);
            }
            PreemptiveExecutionRequest::CatchUp(confirmed_batches) => {
                quiet_unwrap!(
                    self.confirmed_worker
                        .update_messages()
                        .send(ConfirmedUpdateMessage::CatchUp(confirmed_batches.clone()))
                );

                quiet_unwrap!(
                    self.preemptive_worker
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::CatchUp(confirmed_batches))
                );
            }
            PreemptiveExecutionRequest::UpdateBatch(update, _) => {
                quiet_unwrap!(
                    self.confirmed_worker
                        .update_messages()
                        .send(ConfirmedUpdateMessage::UpdateBatch(update, Instant::now()))
                );
            }
            PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstateBatch(update, _) => {
                quiet_unwrap!(self.confirmed_worker.update_messages().send(
                    ConfirmedUpdateMessage::UpdateFinalizedAndGetAppstateBatch(
                        update,
                        Instant::now()
                    )
                ));
            }
            PreemptiveExecutionRequest::PreemptiveUpdate(reqs, time) => {
                quiet_unwrap!(
                    self.preemptive_worker
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::PreemptiveUpdate(reqs))
                );
            }
            PreemptiveExecutionRequest::UpdateFinalized(seq_no) => {
                quiet_unwrap!(
                    self.preemptive_worker
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::ConfirmedUpdate(seq_no))
                );
            }
            PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstate(seq_no) => {
                quiet_unwrap!(
                    self.preemptive_worker
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::ConfirmedUpdateEmitAppState(seq_no))
                );

                // The preemptive worker will then forward this request of app state to the confirmed worker
                // when it sends it. The confirmed worker will then directly send the app state message
                // To the state transfer module via the channel.
            }
            PreemptiveExecutionRequest::ExecuteUnordered(unordered_batch) => {
                quiet_unwrap!(
                    self.confirmed_worker
                        .update_messages()
                        .send(ConfirmedUpdateMessage::ExecuteUnordered(unordered_batch))
                )
            }
        }
    }
}
