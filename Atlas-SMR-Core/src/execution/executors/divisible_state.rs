use crate::execution::reply::ReplyNode;
use crate::execution::TExecutor;
use crate::SMRReply;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_smr_application::app::{Application, Request};
use atlas_smr_application::state::divisible_state;
use atlas_smr_application::state::divisible_state::DivisibleState;
use std::sync::Arc;

pub type DVStateInstallHandle<S> = (
    ChannelSyncTx<divisible_state::InstallStateMessage<S>>,
    ChannelSyncRx<divisible_state::AppStateMessage<S>>,
);

pub trait TDivisibleStateExecutor<A, S, NT>: TExecutor<A, S>
where
    A: Application<S> + 'static,
    S: DivisibleState + 'static,
    NT: 'static,
{
    /// Initialization method for the execution
    /// Should return a channel for the state messages to be sent to the execution
    /// As well as a channel to receive checkpoints from the application
    fn init(
        handle: Self::ExecutionHandle,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> atlas_common::error::Result<DVStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static;
}
