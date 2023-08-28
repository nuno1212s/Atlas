use std::sync::Arc;
use std::time::Instant;
use log::info;
use atlas_common::error::*;
use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_core::smr::exec::ReplyNode;
use atlas_execution::app::{Application, BatchReplies, Reply, Request};
use atlas_execution::{ExecutionRequest, ExecutorHandle};
use atlas_execution::state::monolithic_state::{AppStateMessage, InstallStateMessage, MonolithicState};
use atlas_metrics::metrics::metric_duration;
use crate::executable::ExecutorReplier;
use crate::metric::{EXECUTION_LATENCY_TIME_ID, EXECUTION_TIME_TAKEN_ID};
use crate::server::client_replier::ReplyHandle;

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;

pub struct MonolithicExecutor<S, A, NT>
    where S: MonolithicState + 'static,
          A: Application<S> + 'static {
    application: A,
    state: S,

    work_rx: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,

    reply_worker: ReplyHandle<A::AppData>,
    send_node: Arc<NT>,
}

impl<S, A, NT> MonolithicExecutor<S, A, NT>
    where S: MonolithicState + 'static,
          A: Application<S> + 'static + Send,
          NT: 'static {
    pub fn init_handle() -> (ExecutorHandle<A::AppData>, ChannelSyncRx<ExecutionRequest<Request<A, S>>>) {
        let (tx, rx) = channel::new_bounded_sync(EXECUTING_BUFFER);

        (ExecutorHandle::new(tx), rx)
    }

    pub fn init<T>(
        reply_worker: ReplyHandle<A::AppData>,
        handle: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        mut service: A,
        send_node: Arc<NT>)
        -> Result<(ChannelSyncTx<InstallStateMessage<S>>, ChannelSyncRx<AppStateMessage<S>>)>
        where T: ExecutorReplier + 'static,
              NT: ReplyNode<A::AppData> {
        let (state, requests) = if let Some(state) = initial_state {
            state
        } else {
            (A::initial_state()?, vec![])
        };

        let (state_tx, state_rx) = channel::new_bounded_sync(STATE_BUFFER);

        let (checkpoint_tx, checkpoint_rx) = channel::new_bounded_sync(STATE_BUFFER);

        let mut executor = MonolithicExecutor {
            application: service,
            state,
            work_rx: handle,
            state_rx,
            checkpoint_tx,
            reply_worker,
            send_node,
        };


        for request in requests {
            executor.application.update(&mut executor.state, request);
        }

        std::thread::Builder::new()
            .name(format!("Executor thread"))
            .spawn(move || {
                while let Ok(exec_req) = executor.work_rx.recv() {
                    match exec_req {
                        ExecutionRequest::PollStateChannel => {
                            if let Ok(state_recvd) = executor.state_rx.recv() {
                                executor.state = state_recvd.into_state();
                            }
                        }
                        ExecutionRequest::CatchUp(requests) => {
                            info!("Catching up with {} requests", requests.len());

                            for req in requests {
                                executor.application.update(&mut executor.state, req);
                            }
                        }
                        ExecutionRequest::Update((batch, instant)) => {
                            let seq_no = batch.sequence_number();

                            metric_duration(EXECUTION_LATENCY_TIME_ID, instant.elapsed());

                            let start = Instant::now();

                            let reply_batch =
                                executor.application.update_batch(&mut executor.state, batch);

                            metric_duration(EXECUTION_TIME_TAKEN_ID, start.elapsed());

                            // deliver replies
                            executor.execution_finished::<T>(Some(seq_no), reply_batch);
                        }
                        ExecutionRequest::UpdateAndGetAppstate((batch, instant)) => {
                            let seq_no = batch.sequence_number();

                            metric_duration(EXECUTION_LATENCY_TIME_ID, instant.elapsed());

                            let start = Instant::now();

                            let reply_batch =
                                executor.application.update_batch(&mut executor.state, batch);

                            metric_duration(EXECUTION_TIME_TAKEN_ID, start.elapsed());

                            // deliver checkpoint state to the replica
                            executor.deliver_checkpoint_state(seq_no);

                            // deliver replies
                            executor.execution_finished::<T>(Some(seq_no), reply_batch);
                        }
                        ExecutionRequest::Read(_peer_id) => {
                            todo!()
                        }
                        ExecutionRequest::ExecuteUnordered(batch) => {
                            let reply_batch =
                                executor.application.unordered_batched_execution(&executor.state, batch);

                            executor.execution_finished::<T>(None, reply_batch);
                        }
                    }
                }
            })
            .expect("Failed to start executor thread");

        Ok((state_tx, checkpoint_rx))
    }


    ///Clones the current state and delivers it to the application
    /// Takes a sequence number, which corresponds to the last executed consensus instance before we performed the checkpoint
    fn deliver_checkpoint_state(&self, seq: SeqNo) {
        let cloned_state = self.state.clone();

        self.checkpoint_tx.send(AppStateMessage::new(seq, cloned_state)).expect("Failed to send checkpoint");
    }

    fn execution_finished<T>(&self, seq: Option<SeqNo>, batch: BatchReplies<Reply<A, S>>)
        where NT: ReplyNode<A::AppData> + 'static,
              T: ExecutorReplier + 'static {
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

        T::execution_finished::<A::AppData, NT>(send_node, seq, batch);
    }
}