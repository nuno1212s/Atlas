use crate::metric::EXECUTION_LATENCY_TIME_ID;
use crate::scalable::{sc_execute_op_batch, sc_execute_unordered_op_batch, CRUDState, ScalableApp};
use crate::{ExecutorHandles, ExecutorReplier, MonStateInstallHandle};
use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::ordering::SeqNo;
use atlas_metrics::metrics::metric_duration;
use atlas_smr_application::app::{
    AppData, Application, BatchReplies, Reply, Request, UnorderedBatch, UpdateBatch,
};
use atlas_smr_application::state::monolithic_state::{
    AppStateMessage, InstallStateMessage, MonolithicState,
};
use atlas_smr_application::{ExecutionRequest, ExecutorHandle};
use atlas_smr_core::exec::ReplyNode;
use atlas_smr_core::SMRReply;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::Arc;

use tracing::info;

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;

pub struct ScalableMonolithicExecutor<S, A, NT>
where
    S: MonolithicState + 'static + Send + Sync,
    A: Application<S> + 'static,
{
    application: A,
    state: S,

    work_rx: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,

    thread_pool: ThreadPool,

    send_node: Arc<NT>,
}

impl<S, A, NT> ScalableMonolithicExecutor<S, A, NT>
where
    S: MonolithicState + CRUDState + 'static + Send + Sync,
    A: ScalableApp<S> + 'static + Send,
    NT: 'static,
{
    pub fn init_handle() -> ExecutorHandles<A, S> {
        let (tx, rx) = channel::sync::new_bounded_sync(
            EXECUTING_BUFFER,
            Some("Scalable Mon Exec Work Channel"),
        );

        (ExecutorHandle::new(tx), rx)
    }

    pub fn init<T>(
        handle: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
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
            (<A as Application<S>>::initial_state()?, vec![])
        };

        let (state_tx, state_rx) = channel::sync::new_bounded_sync(
            STATE_BUFFER,
            Some("Scalable Mon Install State Channel"),
        );

        let (checkpoint_tx, checkpoint_rx) =
            channel::sync::new_bounded_sync(STATE_BUFFER, Some("Scalable Mon App State Message"));

        let mut executor = ScalableMonolithicExecutor {
            application: service,
            state,
            work_rx: handle,
            state_rx,
            checkpoint_tx,
            thread_pool: ThreadPoolBuilder::new().num_threads(4).build()?,
            send_node,
        };

        for request in requests {
            executor.application.update(&mut executor.state, request);
        }

        executor.run::<T>();

        Ok((state_tx, checkpoint_rx))
    }

    fn run<T>(mut self)
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        std::thread::Builder::new()
            .name("Executor Manager Thread".to_string())
            .spawn(move || self.worker::<T>())
            .expect("Failed to start executor thread");
    }

    fn worker<T>(&mut self)
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        while let Ok(exec_req) = self.work_rx.recv() {
            match exec_req {
                ExecutionRequest::PollStateChannel => {
                    if let Ok(state_recvd) = self.state_rx.recv() {
                        self.state = state_recvd.into_state();
                    }
                }
                ExecutionRequest::CatchUp(requests) => {
                    info!("Catching up with {} batches of requests", requests.len());

                    for batch in requests.into_iter() {
                        let (seq_no, reply_batch) = self.execute_op_batch(batch);

                        self.execution_finished::<T>(Some(seq_no), reply_batch);
                    }
                }
                ExecutionRequest::Update((batch, instant)) => {
                    metric_duration(EXECUTION_LATENCY_TIME_ID, instant.elapsed());

                    let (seq_no, reply_batch) = self.execute_op_batch(batch);

                    // deliver replies
                    self.execution_finished::<T>(Some(seq_no), reply_batch);
                }
                ExecutionRequest::UpdateAndGetAppstate((batch, instant)) => {
                    metric_duration(EXECUTION_LATENCY_TIME_ID, instant.elapsed());

                    let (seq_no, reply_batch) = self.execute_op_batch(batch);

                    // deliver replies
                    self.execution_finished::<T>(Some(seq_no), reply_batch);

                    // deliver checkpoint state to the replica
                    self.deliver_checkpoint_state(seq_no);
                }
                ExecutionRequest::Read(_peer_id) => {
                    todo!()
                }
                ExecutionRequest::ExecuteUnordered(batch) => {
                    let reply = self.execute_unordered_op_batch(batch);

                    self.execution_finished::<T>(None, reply);
                }
            }
        }
    }

    #[inline(always)]
    fn execute_unordered_op_batch(
        &mut self,
        batch: UnorderedBatch<Request<A, S>>,
    ) -> BatchReplies<Reply<A, S>> {
        sc_execute_unordered_op_batch(&mut self.thread_pool, &self.application, &self.state, batch)
    }

    #[inline(always)]
    fn execute_op_batch(
        &mut self,
        batch: UpdateBatch<Request<A, S>>,
    ) -> (SeqNo, BatchReplies<Reply<A, S>>) {
        sc_execute_op_batch(
            &mut self.thread_pool,
            &self.application,
            &mut self.state,
            batch,
        )
    }

    ///Clones the current state and delivers it to the application
    /// Takes a sequence number, which corresponds to the last executed consensus instance before we performed the checkpoint
    fn deliver_checkpoint_state(&self, seq: SeqNo) {
        let cloned_state = self.state.clone();

        self.checkpoint_tx
            .send(AppStateMessage::new(seq, cloned_state))
            .expect("Failed to send checkpoint");
    }

    fn execution_finished<T>(&self, seq: Option<SeqNo>, batch: BatchReplies<Reply<A, S>>)
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier + 'static,
    {
        let send_node = self.send_node.clone();

        /*{
            if let Some(seq) = seq {
                if let Some(observer_handle) = &self.observer_handle {
                    //Do not notify of unordered events
                    let observe_event = MessageType::Event(ObserveEventKind::Executed(seq));

                    if let Err(err) = observer_handle.tx().send(observe_event) {
                        error!("{:?}", err);
                    }
                }
            }
        }*/

        T::execution_finished::<AppData<A, S>, NT>(send_node, seq, batch);
    }
}
