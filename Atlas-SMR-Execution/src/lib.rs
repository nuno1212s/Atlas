#![allow(incomplete_features)]
#![feature(specialization)]

use crate::crud_states::CRUDState;
use crate::exec_handle::ExecutorHandle;
use atlas_common::error::*;
use atlas_common::phantom::FPhantom;
use atlas_smr_application::app::{Application, Request};
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_application::state::divisible_state::DivisibleState;
use atlas_smr_application::state::monolithic_state::MonolithicState;
use atlas_smr_core::execution::executors::divisible_state::{
    DVStateInstallHandle, TDivisibleStateExecutor,
};
use atlas_smr_core::execution::executors::monolithic_state::{
    MonStateInstallHandle, TMonolithicStateExecutor,
};
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_core::execution::TExecutor;
use atlas_smr_core::SMRReply;
use crud_states::CRUDApplication;
use std::sync::Arc;
use repliers::ReplicaReplier;

pub mod crud_states;
mod exec_handle;
pub mod metric;
pub mod scalable;
pub mod single_threaded;
pub mod repliers;

pub struct SingleThreadedMonExecutor<NT>(FPhantom<NT>);

pub struct MultiThreadedMonExecutor<NT>(FPhantom<NT>);

pub struct SingleThreadedDivExecutor<NT>(FPhantom<NT>);

pub struct MultiThreadedDivExecutor<NT>(FPhantom<NT>);

impl<A, S, NT> TExecutor<A, S> for SingleThreadedDivExecutor<NT>
where
    A: Application<S> + 'static,
    S: DivisibleState + Send + 'static,
{
    type ExecutionHandle = ExecutorHandle<Request<A, S>>;

    fn init_handle() -> Self::ExecutionHandle {
        single_threaded::divisible_state_exec::DivisibleStateExecutor::<S, A, NT>::init_handle()
    }
}

impl<A, S, NT> TDivisibleStateExecutor<A, S, NT> for SingleThreadedDivExecutor<NT>
where
    A: Application<S> + 'static,
    S: DivisibleState + Send + 'static,
    NT: 'static,
{
    fn init(
        work_receiver: Self::ExecutionHandle,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<DVStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        single_threaded::divisible_state_exec::DivisibleStateExecutor::<S, A, NT>::init::<
            ReplicaReplier,
        >(
            work_receiver.get_request_receiver().clone(),
            initial_state,
            service,
            send_node,
        )
    }
}

impl<A, S, NT> TExecutor<A, S> for MultiThreadedDivExecutor<NT>
where
    A: CRUDApplication<S> + Send + 'static,
    S: DivisibleState + CRUDState + Send + Sync + 'static,
{
    type ExecutionHandle = ExecutorHandle<Request<A, S>>;

    fn init_handle() -> Self::ExecutionHandle {
        scalable::divisible_state_exec::ScalableDivisibleStateExecutor::<S, A, NT>::init_handle()
    }
}

impl<A, S, NT> TDivisibleStateExecutor<A, S, NT> for MultiThreadedDivExecutor<NT>
where
    A: CRUDApplication<S> + Send + 'static,
    S: DivisibleState + CRUDState + Send + Sync + 'static,
    NT: 'static,
{
    fn init(
        work_receiver: Self::ExecutionHandle,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<DVStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        scalable::divisible_state_exec::ScalableDivisibleStateExecutor::<S, A, NT>::init::<
            ReplicaReplier,
        >(
            work_receiver.get_request_receiver().clone(),
            initial_state,
            service,
            send_node,
        )
    }
}

impl<A, S, NT> TExecutor<A, S> for SingleThreadedMonExecutor<NT>
where
    A: Application<S> + 'static,
    S: MonolithicState + 'static,
{
    type ExecutionHandle = ExecutorHandle<Request<A, S>>;

    fn init_handle() -> Self::ExecutionHandle {
        single_threaded::monolithic_executor::MonolithicExecutor::<S, A, NT>::init_handle()
    }
}

impl<A, S, NT> TMonolithicStateExecutor<A, S, NT> for SingleThreadedMonExecutor<NT>
where
    A: Application<S> + 'static,
    S: MonolithicState + 'static,
{
    fn init(
        work_receiver: Self::ExecutionHandle,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<MonStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        single_threaded::monolithic_executor::MonolithicExecutor::<S, A, NT>::init::<ReplicaReplier>(
            work_receiver.get_request_receiver().clone(),
            initial_state,
            service,
            send_node,
        )
    }
}

impl<A, S, NT> TExecutor<A, S> for MultiThreadedMonExecutor<NT>
where
    A: CRUDApplication<S> + 'static,
    S: MonolithicState + CRUDState + Send + Sync + 'static,
{
    type ExecutionHandle = ExecutorHandle<Request<A, S>>;

    fn init_handle() -> Self::ExecutionHandle {
        scalable::monolithic_exec::ScalableMonolithicExecutor::<S, A, NT>::init_handle()
    }
}

impl<A, S, NT> TMonolithicStateExecutor<A, S, NT> for MultiThreadedMonExecutor<NT>
where
    A: CRUDApplication<S> + 'static,
    S: MonolithicState + CRUDState + Send + Sync + 'static,
    NT: 'static,
{
    fn init(
        work_receiver: Self::ExecutionHandle,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<MonStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        scalable::monolithic_exec::ScalableMonolithicExecutor::<S, A, NT>::init::<ReplicaReplier>(
            work_receiver.get_request_receiver().clone(),
            initial_state,
            service,
            send_node,
        )
    }
}