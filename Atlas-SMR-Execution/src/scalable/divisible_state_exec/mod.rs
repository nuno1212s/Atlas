use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::Arc;

use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::SeqNo;
use atlas_metrics::metrics::metric_duration;
use atlas_smr_application::app::{
    AppData, Application, BatchReplies, Reply, Request, UnorderedBatch, UpdateBatch,
};
use atlas_smr_application::state::divisible_state::{
    AppState, AppStateMessage, DivisibleState, DivisibleStateDescriptor, InstallStateMessage,
};
use atlas_smr_application::{ExecutionRequest, ExecutorHandle};
use atlas_smr_core::exec::ReplyNode;
use atlas_smr_core::SMRReply;

use crate::metric::EXECUTION_LATENCY_TIME_ID;
use crate::scalable::{
    sc_execute_op_batch, sc_execute_unordered_op_batch, CRUDState, ScalableApp, THREAD_POOL_THREADS,
};
use crate::{DVStateInstallHandle, ExecutorHandles, ExecutorReplier};

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;

const PARTS_PER_DELIVERY: usize = 4;

pub struct ScalableDivisibleStateExecutor<S, A, NT>
where
    S: DivisibleState + CRUDState + 'static + Send + Sync,
    A: ScalableApp<S> + 'static,
    NT: 'static,
{
    application: A,
    state: S,

    work_rx: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,

    thread_pool: ThreadPool,

    send_node: Arc<NT>,

    last_checkpoint_descriptor: S::StateDescriptor,
}

impl<S, A, NT> ScalableDivisibleStateExecutor<S, A, NT>
where
    S: DivisibleState + CRUDState + 'static + Sync,
    A: ScalableApp<S> + 'static + Send,
{
    pub fn init_handle() -> ExecutorHandles<A, S> {
        let (tx, rx) =
            channel::sync::new_bounded_sync(EXECUTING_BUFFER, Some("Scalable Work Handle"));

        (ExecutorHandle::new(tx), rx)
    }

    pub fn init<T>(
        handle: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<DVStateInstallHandle<S>>
    where
        T: ExecutorReplier + 'static,
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        let (state, requests) = if let Some(state) = initial_state {
            state
        } else {
            (<A as Application<S>>::initial_state()?, vec![])
        };

        let (state_tx, state_rx) =
            channel::sync::new_bounded_sync(STATE_BUFFER, Some("Install State Work Handle"));

        let (checkpoint_tx, checkpoint_rx) =
            channel::sync::new_bounded_sync(STATE_BUFFER, Some("App State Checkpoint Work Handle"));

        let descriptor = state.get_descriptor().clone();

        let mut executor = ScalableDivisibleStateExecutor {
            application: service,
            state,
            work_rx: handle,
            state_rx,
            checkpoint_tx,
            thread_pool: ThreadPoolBuilder::new()
                .num_threads(THREAD_POOL_THREADS as usize)
                .build()
                .unwrap(),
            send_node,
            last_checkpoint_descriptor: descriptor,
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
            .name("Executor thread".to_string())
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
                    // Receive all state updates that are available
                    while let Ok(state_recvd) = self.state_rx.recv() {
                        match state_recvd {
                            InstallStateMessage::StateDescriptor(_descriptor) => {}
                            InstallStateMessage::StatePart(state_part) => {
                                self.state
                                    .accept_parts(state_part.into_vec())
                                    .expect("Failed to install state parts into executor");
                            }
                            InstallStateMessage::Done => break,
                        }
                    }
                }
                ExecutionRequest::CatchUp(requests) => {
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
    fn deliver_checkpoint_state(&mut self, seq: SeqNo) {
        let current_state = self
            .state
            .prepare_checkpoint()
            .expect("Failed to prepare state checkpoint")
            .clone();

        let diff = self
            .last_checkpoint_descriptor
            .compare_descriptors(&current_state);

        self.checkpoint_tx
            .send(AppStateMessage::new(
                seq,
                AppState::StateDescriptor(current_state),
            ))
            .unwrap();

        for chunk in diff.chunks(PARTS_PER_DELIVERY) {
            let parts = self
                .state
                .get_parts(chunk)
                .expect("Failed to get necessary parts");

            self.checkpoint_tx
                .send(AppStateMessage::new(
                    seq,
                    AppState::StatePart(MaybeVec::Mult(parts)),
                ))
                .unwrap();
        }

        self.checkpoint_tx
            .send(AppStateMessage::new(seq, AppState::Done))
            .expect("Failed to send checkpoint");
    }

    fn execution_finished<T>(&self, seq: Option<SeqNo>, batch: BatchReplies<Reply<A, S>>)
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
        T: ExecutorReplier + 'static,
    {
        let send_node = self.send_node.clone();

        T::execution_finished::<AppData<A, S>, NT>(send_node, seq, batch);
    }
}
