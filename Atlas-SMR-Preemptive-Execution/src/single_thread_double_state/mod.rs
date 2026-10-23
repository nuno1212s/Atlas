use crate::exec_handle::{PreemptiveExecutionRequest, PreemptiveExecutorHandle};
use crate::single_thread_double_state::confirmed_worker::comm_handles::{
    ConfirmedUpdateMessage, ConfirmedWorkerHandle,
};
use crate::single_thread_double_state::preemptive_worker::comm_handles::{
    PreemptiveWorkMessage, PreemptiveWorkerHandle,
};
use crate::single_thread_double_state::state_management::StateMessage;
use atlas_common::channel::{self, NoRetChannelErr, RecvError, sync::ChannelSyncRx};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::{error, quiet_unwrap};
use atlas_smr_application::{
    app::{Application, Request},
    state::monolithic_state::{InstallStateMessage, MonolithicState},
};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::executors::monolithic_state::MonStateInstallHandle;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::repliers::ExecutorReplier;
use std::ops::ControlFlow;
use std::sync::Arc;
use tracing::error;

mod comm_handles;
mod confirmed_worker;
mod duplicate_state;
mod preemptive_worker;
mod state_management;

#[cfg(test)]
pub mod tests {
    /// End-to-end tests driving the preemptive/confirmed worker pair.
    mod execution;
    pub mod test_fixtures;
}

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;

#[derive(Debug)]
enum RunMode {
    Normal,
    StateTransfer,
}

impl RunMode {
    /// How a `sync_select!` arm body ends, given the run mode after its message.
    ///
    /// A round drains every channel it visits, so a body that switched the mode would
    /// otherwise keep being handed the messages queued behind it, and handle them in
    /// Normal mode before the state transfer they must follow. Breaking leaves them
    /// queued for the mode the loop switches to.
    fn round_step(&self) -> ControlFlow<()> {
        match self {
            RunMode::Normal => ControlFlow::Continue(()),
            RunMode::StateTransfer => ControlFlow::Break(()),
        }
    }
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

    #[allow(dead_code)]
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

        let state = if let Some((mut state, requests)) = initial_state {
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
            state.clone(),
            wrapped_application.clone(),
            send_node.clone(),
            confirmed_worker_shared_channels,
        );
        let preemptive_state_handle =
            preemptive_worker::initialize_preemptive_execution::<A, S, NT, T>(
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

    #[allow(clippy::extra_unused_type_parameters)]
    fn worker<T>(&mut self)
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        loop {
            match &self.run_mode {
                RunMode::Normal => {
                    let received = channel::sync::sync_select! {
                        recv(self.work_rx) -> exec_req => {
                            self.handle_preemptive_execution_request(exec_req);

                            // Stop at a `PollStateChannel`: what is queued behind it
                            // must not reach the workers before the state does.
                            Ok::<_, RecvError>(self.run_mode.round_step())
                        }
                    };

                    if received.is_err() {
                        // The work channel has disconnected — the executor
                        // handle was dropped, so the executor is shutting
                        // down. Return to terminate this thread cleanly
                        // instead of spinning on the closed channel.
                        return;
                    }
                }
                RunMode::StateTransfer => {
                    let message = quiet_unwrap!(self.state_rx.recv());

                    let seq = message.sequence_number();
                    let state = message.into_state();

                    // The confirmed worker installs first, and the preemptive worker only
                    // once that has finished. See `send_state_to_confirmed_worker`.
                    quiet_unwrap!(self.send_state_to_confirmed_worker(seq, state.clone()));
                    quiet_unwrap!(self.send_state_to_preemptive_worker(seq, state));

                    self.set_run_mode(RunMode::Normal);
                }
            }
        }
    }

    /// Install `state` on the confirmed worker and wait until it has done so.
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
            .send(StateMessage::ConfirmedStateReceived(seq, state))?;

        self.await_confirmed_baseline(Some(seq))
    }

    /// Wait until the confirmed worker has finished a state install or a catch-up.
    ///
    /// Both move the confirmed worker to a new baseline. The confirmed worker gets that
    /// baseline from here, but the confirmations that follow it from the preemptive worker,
    /// on a different channel and from a different thread, and nothing orders the two. If
    /// the preemptive worker were given the state or batches first, it could confirm the
    /// next batch before the confirmed worker had applied them. A fresh confirmed worker
    /// would then adopt that batch as its baseline and apply it to the old state, diverging
    /// and emitting a wrong checkpoint. So the preemptive worker is only told once this
    /// returns: until then, no such confirmation can exist.
    ///
    /// `expected` is the sequence number the confirmed worker should report, when known.
    fn await_confirmed_baseline(&self, expected: Option<SeqNo>) -> Result<(), NoRetChannelErr> {
        let acked = self.confirmed_worker.baseline_ack_rx().recv()?;

        if let Some(expected) = expected
            && acked != expected
        {
            error!("Confirmed worker acknowledged baseline {acked:?}, expected {expected:?}");
        }

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
                let last_seq = confirmed_batches.iter().last().map(|batch| batch.seq_no());

                quiet_unwrap!(
                    self.confirmed_worker
                        .update_messages()
                        .send(ConfirmedUpdateMessage::CatchUp(confirmed_batches.clone()))
                );

                // The confirmed worker applies the batches first, and the preemptive worker
                // gets them only once it has. See `await_confirmed_baseline`.
                quiet_unwrap!(self.await_confirmed_baseline(last_seq));

                quiet_unwrap!(
                    self.preemptive_worker
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::CatchUp(confirmed_batches))
                );
            }
            PreemptiveExecutionRequest::UpdateBatch(update, instant) => {
                quiet_unwrap!(
                    self.preemptive_worker
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::ConfirmedUpdate(update, instant))
                );
            }
            PreemptiveExecutionRequest::UpdateBatchAndGetAppstate(update, instant) => {
                quiet_unwrap!(self.preemptive_worker.preemptive_exec_handle().send(
                    PreemptiveWorkMessage::ConfirmedUpdateAndGetAppstate(update, instant)
                ));
            }
            PreemptiveExecutionRequest::PreemptiveUpdate(reqs, _time) => {
                quiet_unwrap!(
                    self.preemptive_worker
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::PreemptiveUpdate(reqs))
                );
            }
            PreemptiveExecutionRequest::PreemptiveUpdateFinalized(seq_no, instant) => {
                quiet_unwrap!(self.preemptive_worker.preemptive_exec_handle().send(
                    PreemptiveWorkMessage::PreemptiveUpdateConfirmed(seq_no, instant)
                ));
            }
            PreemptiveExecutionRequest::PreemptiveUpdateFinalizedAndGetAppstate(
                seq_no,
                instant,
            ) => {
                quiet_unwrap!(self.preemptive_worker.preemptive_exec_handle().send(
                    PreemptiveWorkMessage::PreemptiveUpdateConfirmedAndGetAppState(seq_no, instant)
                ));

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
