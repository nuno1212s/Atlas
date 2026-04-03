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
use atlas_smr_execution::repliers::ReplicaReplier;
use std::sync::Arc;

mod exec_handle;
pub mod metric;
mod single_thread_double_state;
mod single_threaded_crud;

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
