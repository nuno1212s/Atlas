use std::sync::Arc;

use crate::exec_handle::{PreemptiveExecutionRequest, PreemptiveExecutorHandle};
use crate::single_thread_double_state::confirmed_worker::confirmed_requests::ConfirmedRequestPipeline;
use crate::single_thread_double_state::state_management::{
    PreemptiveStateManagementHandle, PreemptiveStateMessage, PreemptiveToConfirmedMsg,
};
use atlas_common::channel::{
    self,
    sync::{ChannelSyncRx, ChannelSyncTx},
};
use atlas_common::error::Result;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::{quiet_unwrap, unwrap_channel};
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use atlas_smr_application::{
    app::{Application, Request},
    state::monolithic_state::{AppStateMessage, InstallStateMessage, MonolithicState},
};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::executors::monolithic_state::MonStateInstallHandle;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::repliers::ExecutorReplier;
use rayon::{ThreadPool, ThreadPoolBuilder};
use crate::single_thread_double_state::preemptive_worker::comm_handles::PreemptiveWorkMessage;

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
    application: Arc<A>,

    run_mode: RunMode,

    confirmed_state: ConfirmedRequestPipeline<S>,
    confirmed_state: ,
    preemptive_state: PreemptiveStateManagementHandle<Request<A, S>, S>,

    read_thread_pool: ThreadPool,

    work_rx: ChannelSyncRx<PreemptiveExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,

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
    ) -> Result<MonStateInstallHandle<S>>
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        let wrapped_application = Arc::new(service);

        let state = if let Some((mut state, requests)) = initial_state {
            for request in requests {
                wrapped_application.update(&mut state, request.clone());
            }

            state
        } else {
            A::initial_state()?
        };

        let confirmed_state = ConfirmedRequestPipeline::<S>::new(
            //TODO Handle seq numbers in the initial state
            SeqNo::ZERO,
            state.clone(),
        );

        let (state_tx, state_rx) = channel::sync::new_bounded_sync(
            STATE_BUFFER,
            Some("ST Monolithic Executor Work InstState"),
        );

        let (checkpoint_tx, checkpoint_rx) =
            channel::sync::new_bounded_sync(STATE_BUFFER, Some("ST Monolithic Executor AppState"));

        let preemptive_state_handle =
            preemptive_worker::initialize_preemptive_execution::<A, S, NT, T>(
                SeqNo::ZERO,
                state,
                wrapped_application.clone(),
                send_node.clone(),
            );

        let mut executor = Self {
            application: wrapped_application,
            run_mode: RunMode::Normal,
            confirmed_state,
            preemptive_state: preemptive_state_handle,
            read_thread_pool: ThreadPoolBuilder::new().num_threads(4).build()?,
            work_rx: handle,
            state_rx,
            checkpoint_tx,
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
                    recv(unwrap_channel!(self.preemptive_state.confirmed_updates_rx())) -> update_msg => {
                        if let Ok(update_msg) = update_msg {
                            self.handle_confirmed_update(update_msg);
                        }
                    },
                     recv(unwrap_channel!(self.work_rx)) -> exec_req => {
                        if let Ok(exec_req) = exec_req {
                            self.handle_preemptive_execution_request::<T>(exec_req);
                        }
                    }
                }
            }
            RunMode::StateTransfer => {
                let message = quiet_unwrap!(self.state_rx.recv());

                self.handle_state_install_message(message);
            }
        }
    }

    fn handle_preemptive_execution_request<T>(
        &mut self,
        execution_request: PreemptiveExecutionRequest<Request<A, S>>,
    ) where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        match execution_request {
            PreemptiveExecutionRequest::PollStateChannel => {
                self.set_run_mode(RunMode::StateTransfer);
            }
            PreemptiveExecutionRequest::CatchUp(confirmed_batches) => {
                self.handle_catch_up::<T>(confirmed_batches);
            }
            PreemptiveExecutionRequest::UpdateBatch(_, _) => {
                todo!("Handle directly sent update batches");
            }
            PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstateBatch(_, _) => {
                todo!("Handle directly sent update batches with appstate retrieval");
            }
            PreemptiveExecutionRequest::PreemptiveUpdate(reqs, time) => {
                quiet_unwrap!(
                    self.preemptive_state
                    .preemptive_exec_handle()
                    .send(PreemptiveWorkMessage::PreemptiveUpdate(reqs))
                );
            }
            PreemptiveExecutionRequest::UpdateFinalized(seq_no) => {
                quiet_unwrap!(
                    self.preemptive_state
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::ConfirmedUpdate(seq_no))
                );
            }
            PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstate(seq_no) => {
                quiet_unwrap!(
                    self.preemptive_state
                        .preemptive_exec_handle()
                        .send(PreemptiveWorkMessage::ConfirmedUpdate(seq_no))
                );

                todo!("Now we need to get the app state and send it to the executor")
            }
            PreemptiveExecutionRequest::ExecuteUnordered(unordered_batch) => {
                // Unordered batches should be executed on the confirmed state
                // As we only want to return confirmed information which can not be rolled back
                self.handle_unordered_batch::<T>(unordered_batch);
            }
        }
    }

    fn handle_confirmed_update(&mut self, update_msg: PreemptiveToConfirmedMsg<Request<A, S>>) {
        match update_msg {
            PreemptiveToConfirmedMsg::UpdateConfirmed(confirmed_update) => {
                self.confirmed_state
                    .execute_update(&*self.application, confirmed_update);
            }
            PreemptiveToConfirmedMsg::RequestStateCopy(_) => {
                let (seq_no, state) = self.confirmed_state.take_state_snapshot();

                quiet_unwrap!(self.preemptive_state.preemptive_state_handle().send(
                    PreemptiveStateMessage::ConfirmedStateReceived(seq_no, state)
                ));
            }
        }
    }

    fn handle_state_install_message(&mut self, message: InstallStateMessage<S>) {
        let seq = message.sequence_number();

        let s = message.into_state();

        self.confirmed_state.install_state_message(seq, s.clone());

        quiet_unwrap!(
            self.preemptive_state
                .preemptive_state_handle()
                .send(PreemptiveStateMessage::ConfirmedStateReceived(seq, s))
        );

        self.set_run_mode(RunMode::Normal);
    }

    fn handle_catch_up<T>(&mut self, confirmed_batches: MaybeVec<UpdateBatch<Request<A, S>>>)
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        for batch in confirmed_batches {
            let seq = batch.seq_no();

            let batch_replies = self
                .confirmed_state
                .execute_update(&*self.application, batch);

            T::execution_finished::<A::AppData, NT>(
                self.send_node.clone(),
                Some(seq),
                batch_replies,
            );
        }

        let (seq_no, state) = self.confirmed_state.take_state_snapshot();

        quiet_unwrap!(self.preemptive_state.preemptive_state_handle().send(
            PreemptiveStateMessage::ConfirmedStateReceived(seq_no, state)
        ));
    }

    fn handle_unordered_batch<T>(&self, unordered_batch: UnorderedUpdateBatch<Request<A, S>>)
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        // Unordered batches should be executed on the confirmed state
        // As we only want to return confirmed information which can not be rolled bac
        let replies = self.confirmed_state.execute_read(
            &*self.application,
            unordered_batch,
            &self.read_thread_pool,
        );

        T::execution_finished::<A::AppData, NT>(self.send_node.clone(), None, replies);
    }
}
