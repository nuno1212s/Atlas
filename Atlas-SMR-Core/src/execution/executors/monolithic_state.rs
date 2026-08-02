use crate::SMRReply;
use crate::execution::TExecutor;
use crate::execution::reply::ReplyNode;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_smr_application::app::{Application, Request};
use atlas_smr_application::state::monolithic_state;
use atlas_smr_application::state::monolithic_state::MonolithicState;
use std::sync::Arc;

pub type MonStateInstallHandle<S> = (
    ChannelSyncTx<monolithic_state::InstallStateMessage<S>>,
    ChannelSyncRx<monolithic_state::AppStateMessage<S>>,
);

pub trait TMonolithicStateExecutor<A, S, NT>: TExecutor<A, S>
where
    A: Application<S> + 'static,
    S: MonolithicState + 'static,
{
    /// Initialization method for the execution
    /// Should return a channel for the state messages to be sent to the execution
    /// As well as a channel to receive checkpoints from the application
    fn init(
        handle: Self::ExecutionHandle,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> atlas_common::error::Result<MonStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static;
}
