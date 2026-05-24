use crate::exec_handle::PreemptiveExecutorHandle;
use crate::single_thread_double_state::PreemptiveDuplicateStateMonolithicExecutor;
use atlas_smr_application::app::{Application, Request};
use atlas_smr_application::state::monolithic_state::MonolithicState;
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::TExecutor;
use atlas_smr_core::execution::executors::monolithic_state::{
    MonStateInstallHandle, TMonolithicStateExecutor,
};
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::crud_states::{CRUDApplication, CRUDState};
use atlas_smr_execution::repliers::ReplicaReplier;
use std::sync::Arc;

mod exec_handle;
pub mod metric;
mod single_thread_double_state;
mod single_threaded_crud;

/// Dual-state preemptive executor: maintains two full state copies (speculative + confirmed).
/// Requires only `Application<S>`; no CRUD interface needed.
pub struct MonolithicPreemptiveExecutor;

impl<A, S> TExecutor<A, S> for MonolithicPreemptiveExecutor
where
    S: MonolithicState,
    A: Application<S>,
{
    type ExecutionHandle = PreemptiveExecutorHandle<Request<A, S>>;

    fn init_handle() -> Self::ExecutionHandle {
        single_thread_double_state::init_handle::<A, S>()
    }
}

impl<A, S, NT> TMonolithicStateExecutor<A, S, NT> for MonolithicPreemptiveExecutor
where
    S: MonolithicState + 'static + Sync + Send,
    A: Application<S> + 'static + Send + Clone,
{
    fn init(
        handle: Self::ExecutionHandle,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> atlas_common::error::Result<MonStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        PreemptiveDuplicateStateMonolithicExecutor::<S, A, NT>::init::<ReplicaReplier>(
            handle.get_request_receiver().clone(),
            initial_state,
            service,
            send_node,
        )
    }
}

/// Cache-based sequential preemptive executor.
///
/// Maintains a single confirmed state plus an in-memory accumulated cache of pending writes.
/// Each preemptive update executes through a `CachingState` proxy (writes go to the cache only).
/// On confirmation the delta is flushed to the real state — no re-execution required.
/// Backtracking discards pending deltas and rebuilds the cache — no state clone required.
///
/// Requires `S: CRUDState` and `A: CRUDApplication<S>`.
pub struct CRUDMonolithicPreemptiveExecutor;

impl<A, S> TExecutor<A, S> for CRUDMonolithicPreemptiveExecutor
where
    S: MonolithicState + CRUDState,
    A: CRUDApplication<S>,
{
    type ExecutionHandle = PreemptiveExecutorHandle<Request<A, S>>;

    fn init_handle() -> Self::ExecutionHandle {
        single_threaded_crud::init_handle::<A, S>()
    }
}

impl<A, S, NT> TMonolithicStateExecutor<A, S, NT> for CRUDMonolithicPreemptiveExecutor
where
    S: MonolithicState + CRUDState + Clone + Sync + Send + 'static,
    A: CRUDApplication<S> + Send + Sync + Clone + 'static,
{
    fn init(
        handle: Self::ExecutionHandle,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> atlas_common::error::Result<MonStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        single_threaded_crud::init_executor::<A, S, NT, ReplicaReplier>(
            handle.get_request_receiver().clone(),
            initial_state,
            service,
            send_node,
        )
    }
}
