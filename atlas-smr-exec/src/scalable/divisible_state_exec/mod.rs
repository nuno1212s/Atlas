use std::sync::Arc;
use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_execution::app::{Application, Request};
use atlas_execution::{ExecutionRequest, ExecutorHandle};
use atlas_execution::state::divisible_state::{AppStateMessage, DivisibleState, InstallStateMessage};
use crate::scalable::CRUDState;

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;

pub struct ScalableDivisibleStateExecutor<S, A, NT>
    where S: DivisibleState + CRUDState + 'static,
          A: Application<S> + 'static,
          NT: 'static {
    application: A,
    state: S,

    work_rx: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,

    send_node: Arc<NT>,

    last_checkpoint_descriptor: S::StateDescriptor,
}

impl<S, A, NT> ScalableDivisibleStateExecutor<S, A, NT>
    where S: DivisibleState + CRUDState + 'static + Send,
          A: Application<S> + 'static + Send {

    pub fn init_handle() -> (ExecutorHandle<A::AppData>, ChannelSyncRx<ExecutionRequest<Request<A, S>>>) {
        let (tx, rx) = channel::new_bounded_sync(EXECUTING_BUFFER);

        (ExecutorHandle::new(tx), rx)
    }


}