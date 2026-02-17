use std::ops::DerefMut;
use std::sync::Arc;

use crate::{
    exec_handle::{PreemptiveExecutionRequest, PreemptiveExecutorHandle},
    single_thread_double_state::duplicate_state::DuplicateState,
};
use atlas_common::channel::{
    self,
    sync::{ChannelSyncRx, ChannelSyncTx},
};
use atlas_common::error::Result;
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

    state: DuplicateState<S>,

    read_thread_pool: ThreadPool,

    work_rx: ChannelSyncRx<PreemptiveExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,

    send_node: Arc<NT>,
}

impl<S, A, NT> PreemptiveDuplicateStateMonolithicExecutor<S, A, NT>
where
    S: MonolithicState + 'static,
    A: Application<S> + 'static + Send,
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
        let (state, requests) = if let Some(state) = initial_state {
            state
        } else {
            (A::initial_state()?, vec![])
        };

        let duplicate_state = DuplicateState::new(state);

        let (state_tx, state_rx) = channel::sync::new_bounded_sync(
            STATE_BUFFER,
            Some("ST Monolithic Executor Work InstState"),
        );

        let (checkpoint_tx, checkpoint_rx) =
            channel::sync::new_bounded_sync(STATE_BUFFER, Some("ST Monolithic Executor AppState"));

        let mut executor = Self {
            application: service,
            state: duplicate_state,
            read_thread_pool: ThreadPoolBuilder::new().num_threads(4).build()?,
            work_rx: handle,
            state_rx,
            checkpoint_tx,
            send_node,
        };

        {
            let mut state_handle = executor.state.confirmed_state();

            for request in requests {
                executor
                    .application
                    .update(state_handle.deref_mut(), request);
            }
        }

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
                PreemptiveExecutionRequest::CatchUp(_) => {}
                PreemptiveExecutionRequest::UpdateBatch(_, _) => {}
                PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstateBatch(_, _) => {}
                PreemptiveExecutionRequest::PreemptiveUpdate(_, _) => {}
                PreemptiveExecutionRequest::UpdateFinalized(_) => {}
                PreemptiveExecutionRequest::UpdateFinalizedAndGetAppstate(_) => {}
                PreemptiveExecutionRequest::ExecuteUnordered(_) => {}
                PreemptiveExecutionRequest::Read(_) => {}
            }
        }
        // Worker loop implementation goes here
    }
}
