use std::sync::Arc;
use std::time::Instant;
use atlas_common::error::*;
use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_core::serialize::{LogTransferMessage, OrderingProtocolMessage, ServiceMsg, StateTransferMessage};
use atlas_execution::app::{Application, BatchReplies, Reply, Request};
use atlas_execution::{ExecutionRequest, ExecutorHandle};
use atlas_execution::state::divisible_state::{AppStateMessage, DivisibleState, DivisibleStateDescriptor, InstallStateMessage};
use atlas_metrics::metrics::metric_duration;
use crate::executable::ExecutorReplier;
use crate::metric::{EXECUTION_LATENCY_TIME_ID, EXECUTION_TIME_TAKEN_ID};
use crate::server::client_replier::ReplyHandle;

const EXECUTING_BUFFER: usize = 16384;
const STATE_BUFFER: usize = 128;

pub struct DivisibleStateExecutor<S, A, NT>
    where S: DivisibleState + 'static,
          A: Application<S> + 'static,
          NT: 'static {
    application: A,
    state: S,

    work_rx: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
    state_rx: ChannelSyncRx<InstallStateMessage<S>>,
    checkpoint_tx: ChannelSyncTx<AppStateMessage<S>>,

    reply_worker: ReplyHandle<A::AppData>,
    send_node: Arc<NT>,

    last_checkpoint_descriptor: S::StateDescriptor,
}

impl<S, A, NT> DivisibleStateExecutor<S, A, NT>
    where S: DivisibleState + 'static + Send,
          A: Application<S> + 'static + Send {
    pub fn init_handle() -> (ExecutorHandle<A::AppData>, ChannelSyncRx<ExecutionRequest<Request<A, S>>>) {
        let (tx, rx) = channel::new_bounded_sync(EXECUTING_BUFFER);

        (ExecutorHandle::new(tx), rx)
    }

    pub fn init<OP, ST, LT, T>(
        reply_worker: ReplyHandle<A::AppData>,
        handle: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        mut service: A,
        send_node: Arc<NT>)
        -> Result<(ChannelSyncTx<InstallStateMessage<S>>, ChannelSyncRx<AppStateMessage<S>>)>
        where OP: OrderingProtocolMessage + 'static,
              ST: StateTransferMessage + 'static,
              LT: LogTransferMessage + 'static,
              T: ExecutorReplier + 'static,
              NT: ProtocolNetworkNode<ServiceMsg<A::AppData, OP, ST, LT>> + 'static {
        let (state, requests) = if let Some(state) = initial_state {
            state
        } else {
            (A::initial_state()?, vec![])
        };

        let (state_tx, state_rx) = channel::new_bounded_sync(STATE_BUFFER);

        let (checkpoint_tx, checkpoint_rx) = channel::new_bounded_sync(STATE_BUFFER);

        let descriptor = state.get_descriptor().clone();

        let mut executor = DivisibleStateExecutor {
            application: service,
            state,
            work_rx: handle,
            state_rx,
            checkpoint_tx,
            reply_worker,
            send_node,
            last_checkpoint_descriptor: descriptor,
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
                            // Receive all state updates that are available
                            while let Ok(state_recvd) = executor.state_rx.recv() {
                                match state_recvd {
                                    InstallStateMessage::StatePart(state_part) => {
                                        executor.state.accept_parts(state_part).expect("Failed to install state parts into executor");
                                    }
                                    InstallStateMessage::Done => break
                                }
                            }
                        }
                        ExecutionRequest::CatchUp(requests) => {
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
                            executor.execution_finished::<OP, ST, LT, T>(Some(seq_no), reply_batch);
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
                            executor.execution_finished::<OP, ST, LT, T>(Some(seq_no), reply_batch);
                        }
                        ExecutionRequest::Read(_peer_id) => {
                            todo!()
                        }
                        ExecutionRequest::ExecuteUnordered(batch) => {
                            let reply_batch =
                                executor.application.unordered_batched_execution(&executor.state, batch);

                            executor.execution_finished::<OP, ST, LT, T>(None, reply_batch);
                        }
                    }
                }
            })
            .expect("Failed to start executor thread");

        Ok((state_tx, checkpoint_rx))
    }

    ///Clones the current state and delivers it to the application
    /// Takes a sequence number, which corresponds to the last executed consensus instance before we performed the checkpoint
    fn deliver_checkpoint_state(&mut self, seq: SeqNo) {
        let current_state = self.state.prepare_checkpoint().expect("Failed to prepare state checkpoint").clone();

        let diff = self.last_checkpoint_descriptor.compare_descriptors(&current_state);

        let parts = self.state.get_parts(&diff).expect("Failed to get necessary parts");

        self.checkpoint_tx.send(AppStateMessage::new(seq, current_state.clone(), parts)).expect("Failed to send checkpoint");
    }

    fn execution_finished<OP, ST, LT, T>(&self, seq: Option<SeqNo>, batch: BatchReplies<Reply<A, S>>)
        where OP: OrderingProtocolMessage + 'static,
              ST: StateTransferMessage + 'static,
              LT: LogTransferMessage + 'static,
              NT: ProtocolNetworkNode<ServiceMsg<A::AppData, OP, ST, LT>> + 'static,
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

        T::execution_finished::<A::AppData, OP, ST, LT, NT>(send_node, seq, batch);
    }
}