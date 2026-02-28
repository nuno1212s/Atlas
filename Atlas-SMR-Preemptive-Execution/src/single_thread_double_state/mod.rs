use std::sync::Arc;

use crate::exec_handle::{PreemptiveExecutionRequest, PreemptiveExecutorHandle};
use crate::single_thread_double_state::confirmed_worker::confirmed_requests::ConfirmedRequestPipeline;
use crate::single_thread_double_state::state_management::{PreemptiveStateManagementHandle, PreemptiveStateMessage, PreemptiveToConfirmedMsg};
use atlas_common::channel::{
    self,
    sync::{ChannelSyncRx, ChannelSyncTx},
};
use atlas_common::error::Result;
use atlas_common::ordering::SeqNo;
use atlas_smr_application::{
    app::{Application, Request},
    state::monolithic_state::{AppStateMessage, InstallStateMessage, MonolithicState},
};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::executors::monolithic_state::MonStateInstallHandle;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::ExecutorReplier;
use rayon::{ThreadPool, ThreadPoolBuilder};

mod confirmed_worker;
mod duplicate_state;
mod preemptive_worker;
mod state_management;

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;

pub struct PreemptiveDuplicateStateMonolithicExecutor<S, A, NT>
where
    S: MonolithicState + 'static,
    A: Application<S> + 'static,
{
    application: A,

    confirmed_state: ConfirmedRequestPipeline<S>,
    preemptive_state: PreemptiveStateManagementHandle<A, S>,

    read_thread_pool: ThreadPool,

    work_rx: ChannelSyncRx<PreemptiveExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,

    send_node: Arc<NT>,
}

impl<S, A, NT> PreemptiveDuplicateStateMonolithicExecutor<S, A, NT>
where
    S: MonolithicState + 'static,
    A: Application<S> + 'static + Send + Clone,
{
    pub fn init_handle() -> PreemptiveExecutorHandle<Request<A, S>> {
        let (tx, rx) = channel::sync::new_bounded_sync(
            EXECUTING_BUFFER,
            Some("ST Preemptive Duplicate State Executor Work Channel"),
        );

        PreemptiveExecutorHandle::new(tx, rx)
    }

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
        let state = if let Some((mut state, requests)) = initial_state {
            for request in requests {
                service.update(&mut state, request.clone());
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

        let preemptive_state_handle = preemptive_worker::initialize_preemptive_execution::<A, S>(
            SeqNo::ZERO,
            state,
            service.clone(),
        );

        let mut executor = Self {
            application: service,
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

    fn worker<T>(&mut self)
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        while let Ok(exec_req) = self.work_rx.recv() {
            match exec_req {
                PreemptiveExecutionRequest::PollStateChannel => {}
                PreemptiveExecutionRequest::CatchUp(confirmed_batches) => {}
                PreemptiveExecutionRequest::UpdateBatch(confirmed_update, time_sent) => {}
                PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstateBatch(_, _) => {}
                PreemptiveExecutionRequest::PreemptiveUpdate(reqs, time) => {
                    self.preemptive_state.preemptive_execution_handle()
                        .send(PreemptiveStateMessage::PreemptiveUpdate(reqs));
                }
                PreemptiveExecutionRequest::UpdateFinalized(seq_no) => {
                        self.preemptive_state.preemptive_execution_handle()
                            .send(PreemptiveStateMessage::ConfirmedUpdate(seq_no));
                }
                PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstate(seq_no) => {
                    self.preemptive_state.preemptive_execution_handle()
                        .send(PreemptiveStateMessage::ConfirmedUpdate(seq_no));
                }
                PreemptiveExecutionRequest::ExecuteUnordered(_) => {}
                PreemptiveExecutionRequest::Read(_) => {}
            }
        }

        // Worker loop implementation goes here
    }

    fn poll_confirmed_updates<T>(&mut self)
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        while let Ok(update_msg) = self.preemptive_state.confirmed_updates_rx().recv() {
            match update_msg {
                PreemptiveToConfirmedMsg::UpdateConfirmed(confirmed_update) => {
                    self.confirmed_state.execute_update(&self.application, confirmed_update);
                }
                PreemptiveToConfirmedMsg::RequestStateCopy(seq_no) => {
                    let (seq_no, state) = self.confirmed_state.take_state_snapshot();

                    self.preemptive_state.preemptive_execution_handle()
                        .send(PreemptiveStateMessage::ConfirmedStateReceived(seq_no, state));
                }
            }
        }
    }

    fn poll_state_channel(&mut self) {
        while let Ok(state) = self.state_rx.recv() {}
    }
}
