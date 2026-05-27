#![allow(dead_code)]

use crate::exec_handle::{PreemptiveExecutionRequest, PreemptiveExecutorHandle};
use crate::scalable_crud::pending_state::{PreemptiveError, ScalableCachingPreemptiveState};
use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::quiet_unwrap;
use atlas_core::execution::requests::{ReplyBatch, UpdateBatch, UpdateReply};
use atlas_smr_application::app::{Reply, Request};
use atlas_smr_application::state::monolithic_state::{
    AppStateMessage, InstallStateMessage, MonolithicState,
};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::executors::monolithic_state::MonStateInstallHandle;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::crud_states::{CRUDApplication, CRUDState};
use atlas_smr_execution::repliers::ExecutorReplier;
use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::Arc;
use tracing::error;

mod execution_unit;
mod pending_state;

#[cfg(test)]
pub mod tests {
    mod integration_tests;
    pub mod test_fixtures;
    mod unit_tests;
}

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;
/// Thread pool size for the parallel speculative execution phase.
const PARALLEL_EXEC_THREAD_POOL_SIZE: usize = 4;
/// Thread pool size for parallel unordered (read-only) execution.
const READ_THREAD_POOL_SIZE: usize = 4;

enum RunMode {
    Normal,
    StateTransfer,
}

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

pub fn init_handle<A, S>() -> PreemptiveExecutorHandle<Request<A, S>>
where
    S: MonolithicState + CRUDState + Sync,
    A: CRUDApplication<S> + Sync,
    Request<A, S>: Clone,
{
    let (tx, rx) = channel::sync::new_bounded_sync(
        EXECUTING_BUFFER,
        Some("Scalable CRUD Preemptive Executor Work Channel"),
    );
    PreemptiveExecutorHandle::new(tx, rx)
}

pub fn init_executor<A, S, NT, T>(
    handle: ChannelSyncRx<PreemptiveExecutionRequest<Request<A, S>>>,
    initial_state: Option<(S, Vec<Request<A, S>>)>,
    service: A,
    send_node: Arc<NT>,
) -> atlas_common::error::Result<MonStateInstallHandle<S>>
where
    A: CRUDApplication<S> + Send + Sync + Clone + 'static,
    S: MonolithicState + CRUDState + Clone + Sync + Send + 'static,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    T: ExecutorReplier + 'static,
    Request<A, S>: Clone,
{
    let application = Arc::new(service);

    let initial = if let Some((mut s, requests)) = initial_state {
        for req in requests {
            application.update(&mut s, req);
        }
        (SeqNo::ZERO, s)
    } else {
        (SeqNo::ZERO, A::initial_state()?)
    };

    let exec_thread_pool = ThreadPoolBuilder::new()
        .num_threads(PARALLEL_EXEC_THREAD_POOL_SIZE)
        .build()?;

    let (state_tx, state_rx) = channel::sync::new_bounded_sync(
        STATE_BUFFER,
        Some("Scalable CRUD Preemptive Executor Install State Channel"),
    );
    let (checkpoint_tx, checkpoint_rx) = channel::sync::new_bounded_sync(
        STATE_BUFFER,
        Some("Scalable CRUD Preemptive Executor App State Channel"),
    );

    let worker = ScalableCachingPreemptiveWorker {
        state: ScalableCachingPreemptiveState::new(initial, exec_thread_pool),
        application,
        node: send_node,
        work_rx: handle,
        state_rx,
        checkpoint_tx,
        read_thread_pool: ThreadPoolBuilder::new()
            .num_threads(READ_THREAD_POOL_SIZE)
            .build()?,
        run_mode: RunMode::Normal,
    };

    worker.spawn::<T>();

    Ok((state_tx, checkpoint_rx))
}

// ---------------------------------------------------------------------------
// Worker internals
// ---------------------------------------------------------------------------

struct ScalableCachingPreemptiveWorker<S, A, NT>
where
    A: CRUDApplication<S>,
    S: CRUDState + Sync,
{
    state: ScalableCachingPreemptiveState<S, A>,
    application: Arc<A>,
    node: Arc<NT>,
    work_rx: ChannelSyncRx<PreemptiveExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,
    read_thread_pool: ThreadPool,
    run_mode: RunMode,
}

#[derive(Debug)]
enum WorkerError {
    ChannelClosed,
}

impl<S, A, NT> ScalableCachingPreemptiveWorker<S, A, NT>
where
    A: CRUDApplication<S> + Send + Sync + 'static,
    S: CRUDState + MonolithicState + Clone + Sync + Send + 'static,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    Request<A, S>: Clone,
{
    fn spawn<T>(mut self)
    where
        T: ExecutorReplier + 'static,
    {
        std::thread::Builder::new()
            .name("Scalable Caching Preemptive Worker".to_string())
            .spawn(move || {
                self.worker_loop::<T>();
            })
            .expect("Failed to spawn scalable caching preemptive worker thread");
    }

    fn worker_loop<T>(&mut self)
    where
        T: ExecutorReplier + 'static,
    {
        loop {
            let result = match self.run_mode {
                RunMode::Normal => self.normal_mode::<T>(),
                RunMode::StateTransfer => self.state_transfer_mode(),
            };

            if let Err(err) = result {
                error!("Scalable caching preemptive worker terminated: {:?}", err);
                break;
            }
        }
    }

    fn normal_mode<T>(&mut self) -> Result<(), WorkerError>
    where
        T: ExecutorReplier + 'static,
    {
        let msg = self
            .work_rx
            .recv()
            .map_err(|_| WorkerError::ChannelClosed)?;
        self.handle_request::<T>(msg);
        Ok(())
    }

    fn state_transfer_mode(&mut self) -> Result<(), WorkerError> {
        let msg = self
            .state_rx
            .recv()
            .map_err(|_| WorkerError::ChannelClosed)?;
        let seq = msg.sequence_number();
        let state = msg.into_state();
        self.state.install_confirmed_state(seq, state);
        self.run_mode = RunMode::Normal;
        Ok(())
    }

    fn handle_request<T>(&mut self, req: PreemptiveExecutionRequest<Request<A, S>>)
    where
        T: ExecutorReplier + 'static,
    {
        match req {
            PreemptiveExecutionRequest::PollStateChannel => {
                self.run_mode = RunMode::StateTransfer;
            }

            PreemptiveExecutionRequest::CatchUp(batches) => {
                let results = self.state.handle_catch_up(&self.application, batches);
                for (seq, replies) in results {
                    T::execution_finished::<A::AppData, NT>(self.node.clone(), Some(seq), replies);
                }
            }

            PreemptiveExecutionRequest::UpdateBatch(batch, _instant) => {
                let seq = batch.sequence_number();
                match self.state.handle_confirmed_update(&self.application, batch) {
                    Ok(replies) => {
                        T::execution_finished::<A::AppData, NT>(
                            self.node.clone(),
                            Some(seq),
                            replies,
                        );
                    }
                    Err(e) => error!("Confirmed update error: {:?}", e),
                }
            }

            PreemptiveExecutionRequest::UpdateBatchAndGetAppstate(batch, _instant) => {
                let seq = batch.sequence_number();
                match self.state.handle_confirmed_update(&self.application, batch) {
                    Ok(replies) => {
                        T::execution_finished::<A::AppData, NT>(
                            self.node.clone(),
                            Some(seq),
                            replies,
                        );
                        self.emit_checkpoint(seq);
                    }
                    Err(e) => error!("Confirmed update+checkpoint error: {:?}", e),
                }
            }

            PreemptiveExecutionRequest::PreemptiveUpdate(batch, _instant) => {
                self.handle_preemptive_update(batch);
            }

            PreemptiveExecutionRequest::PreemptiveUpdateFinalized(seq) => {
                match self.state.handle_update_confirmed(seq) {
                    Ok(replies) => {
                        T::execution_finished::<A::AppData, NT>(
                            self.node.clone(),
                            Some(seq),
                            replies,
                        );
                    }
                    Err(e) => error!("Update confirmation error: {:?}", e),
                }
            }

            PreemptiveExecutionRequest::PreemptiveUpdateFinalizedAndGetAppstate(seq) => {
                match self.state.handle_update_confirmed(seq) {
                    Ok(replies) => {
                        T::execution_finished::<A::AppData, NT>(
                            self.node.clone(),
                            Some(seq),
                            replies,
                        );
                        self.emit_checkpoint(seq);
                    }
                    Err(e) => error!("Update confirmation+checkpoint error: {:?}", e),
                }
            }

            PreemptiveExecutionRequest::ExecuteUnordered(unordered_batch) => {
                let application = self.application.clone();
                let state: &S = self.state.confirmed_state();
                let replies: ReplyBatch<Reply<A, S>> = {
                    let pool: &ThreadPool = &self.read_thread_pool;
                    pool.install(|| {
                        unordered_batch
                            .into_inner()
                            .into_par_iter()
                            .map(|item| {
                                let (info, op) = item.into_inner();
                                let reply = application.unordered_execution(state, op);
                                UpdateReply::new(info, reply)
                            })
                            .collect::<Vec<_>>()
                            .into()
                    })
                };
                T::execution_finished::<A::AppData, NT>(self.node.clone(), None, replies);
            }
        }
    }

    fn handle_preemptive_update(&mut self, batch: UpdateBatch<Request<A, S>>) {
        match self
            .state
            .handle_preemptive_update(&self.application, batch)
        {
            Ok(()) => {}
            Err(PreemptiveError::Backtracking(seq, batch)) => {
                if let Err(e) = self.state.backtrack(seq) {
                    error!("Backtrack failed: {:?}", e);
                    return;
                }
                if let Err(e) = self
                    .state
                    .handle_preemptive_update(&self.application, batch)
                {
                    error!("Preemptive re-execution after backtrack failed: {:?}", e);
                }
            }
            Err(PreemptiveError::FutureRequest(seq, current)) => {
                error!(
                    "Preemptive update at {seq:?} is ahead of current head {current:?}; dropping"
                );
            }
        }
    }

    fn emit_checkpoint(&self, seq: SeqNo) {
        let state = self.state.confirmed_state().clone();
        quiet_unwrap!(self.checkpoint_tx.send(AppStateMessage::new(seq, state)));
    }
}
