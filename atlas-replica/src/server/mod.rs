//! Contains the server side core protocol logic of `febft`.

use std::sync::Arc;
use std::time::{Duration, Instant};
use futures_timer::Delay;

use log::{debug, error, info, trace};
use atlas_common::{channel, threadpool};
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};

use atlas_common::async_runtime as rt;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::{Node, NodeConnections, NodeIncomingRqHandler};
use atlas_communication::message::StoredMessage;
use atlas_execution::app::{Request, Service, State};
use atlas_execution::ExecutorHandle;
use atlas_execution::serialize::{digest_state, SharedData};
use atlas_core::messages::Message;
use atlas_core::messages::SystemMessage;
use atlas_core::ordering_protocol::{ExecutionResult, OrderingProtocol, OrderingProtocolArgs, ProtocolConsensusDecision};
use atlas_core::ordering_protocol::OrderProtocolExecResult;
use atlas_core::ordering_protocol::OrderProtocolPoll;
use atlas_core::persistent_log::{PersistableOrderProtocol, PersistableStateTransferProtocol, StatefulOrderingProtocolLog, WriteMode};
use atlas_core::request_pre_processing::{initialize_request_pre_processor, PreProcessorMessage, RequestPreProcessor};
use atlas_core::request_pre_processing::work_dividers::WDRoundRobin;
use atlas_core::serialize::{OrderingProtocolMessage, ServiceMsg};
use atlas_core::state_transfer::{Checkpoint, StatefulOrderProtocol, StateTransferProtocol, STResult, STTimeoutResult};
use atlas_core::timeouts::{RqTimeout, TimedOut, Timeout, TimeoutKind, Timeouts};
use atlas_metrics::metrics::{metric_duration, metric_increment, metric_store_count};
use atlas_persistent_log::{NoPersistentLog, PersistentLog};
use crate::config::ReplicaConfig;
use crate::executable::{Executor, ReplicaReplier};
use crate::metric::{ORDERING_PROTOCOL_POLL_TIME_ID, ORDERING_PROTOCOL_PROCESS_TIME_ID, REPLICA_INTERNAL_PROCESS_TIME_ID, REPLICA_ORDERED_RQS_PROCESSED_ID, REPLICA_RQ_QUEUE_SIZE_ID, REPLICA_TAKE_FROM_NETWORK_ID, RUN_LATENCY_TIME_ID, STATE_TRANSFER_PROCESS_TIME_ID, TIMEOUT_PROCESS_TIME_ID};
use crate::persistent_log::SMRPersistentLog;
use crate::server::client_replier::Replier;

//pub mod observer;

pub mod client_replier;
pub mod follower_handling;
// pub mod rq_finalizer;


pub const REPLICA_WAIT_TIME: Duration = Duration::from_millis(1000);

#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum ReplicaPhase {
    // The replica is currently executing the ordering protocol
    OrderingProtocol,
    // The replica is currently executing the state transfer protocol
    StateTransferProtocol,
}

pub struct Replica<S, OP, ST, NT, PL> where S: Service + 'static,
                                            OP: StatefulOrderProtocol<S::Data, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + 'static,
                                            ST: StateTransferProtocol<S::Data, OP, NT, PL> + PersistableStateTransferProtocol + 'static,
                                            ST: PersistableStateTransferProtocol,
                                            PL: SMRPersistentLog<S::Data, OP::Serialization, OP::StateSerialization> + 'static, {
    replica_phase: ReplicaPhase,
    // The ordering protocol
    ordering_protocol: OP,
    state_transfer_protocol: ST,
    rq_pre_processor: RequestPreProcessor<Request<S>>,
    timeouts: Timeouts,
    executor_handle: ExecutorHandle<S::Data>,
    // The networking layer for a Node in the network (either Client or Replica)
    node: Arc<NT>,
    // THe handle to the execution and timeouts handler
    execution_rx: ChannelSyncRx<Message<S::Data>>,
    execution_tx: ChannelSyncTx<Message<S::Data>>,
    // THe handle for processed timeouts
    processed_timeout: (ChannelSyncTx<(Vec<RqTimeout>, Vec<RqTimeout>)>, ChannelSyncRx<(Vec<RqTimeout>, Vec<RqTimeout>)>),
    persistent_log: PL,
}

impl<S, OP, ST, NT, PL> Replica<S, OP, ST, NT, PL> where S: Service + 'static,
                                                         OP: StatefulOrderProtocol<S::Data, NT, PL> + 'static + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + Send,
                                                         ST: StateTransferProtocol<S::Data, OP, NT, PL> + 'static + PersistableStateTransferProtocol + Send,
                                                         NT: Node<ServiceMsg<S::Data, OP::Serialization, ST::Serialization>> + 'static,
                                                         PL: SMRPersistentLog<S::Data, OP::Serialization, OP::StateSerialization> + 'static, {
    pub async fn bootstrap(cfg: ReplicaConfig<S, OP, ST, NT, PL>) -> Result<Self> {
        let ReplicaConfig {
            service,
            id: log_node_id,
            n,
            f,
            view,
            next_consensus_seq,
            db_path, op_config,
            st_config,
            node: node_config, ..
        } = cfg;

        debug!("{:?} // Bootstrapping replica, starting with networking", log_node_id);

        let node = NT::bootstrap(node_config).await?;

        let (executor, handle) = Executor::<S, NT>::init_handle();
        let (exec_tx, exec_rx) = channel::new_bounded_sync(1024);

        //CURRENTLY DISABLED, USING THREADPOOL INSTEAD
        let reply_handle = Replier::new(node.id(), node.clone());

        debug!("{:?} // Initializing timeouts", log_node_id);

        let (rq_pre_processor, batch_input) = initialize_request_pre_processor
            ::<WDRoundRobin, S::Data, OP::Serialization, ST::Serialization, NT>(4, node.clone());

        let default_timeout = Duration::from_secs(3);

        // start timeouts handler
        let timeouts = Timeouts::new::<S::Data>(log_node_id.clone(), Duration::from_millis(1),
                                                default_timeout,
                                                exec_tx.clone());

        //Calculate the initial state of the application so we can pass it to the ordering protocol
        let init_ex_state = S::initial_state()?;
        let digest = digest_state::<S::Data>(&init_ex_state)?;

        let initial_state = Checkpoint::new(SeqNo::ZERO, init_ex_state, digest);

        let persistent_log = PL::init_log::<String, NoPersistentLog, OP, ST>(executor.clone(), db_path)?;

        let log = persistent_log.read_state(WriteMode::BlockingSync)?;

        let op_args = OrderingProtocolArgs(executor.clone(), timeouts.clone(),
                                           rq_pre_processor.clone(),
                                           batch_input, node.clone(), persistent_log.clone());

        let ordering_protocol = if let Some((view, log)) = log {
            // Initialize the ordering protocol
            OP::initialize_with_initial_state(op_config, op_args, log)?
        } else {
            OP::initialize(op_config, op_args)?
        };

        let state_transfer_protocol = ST::initialize(st_config, timeouts.clone(), node.clone(), persistent_log.clone())?;

        // Check with the order protocol to see if there were no stored states
        let state = None;

        // start executor
        Executor::<S, NT>::new::<OP::Serialization, ST::Serialization, ReplicaReplier>(
            reply_handle,
            handle,
            service,
            state,
            node.clone(),
            exec_tx.clone(),
        )?;

        info!("{:?} // Connecting to other replicas.", log_node_id);

        let mut connections = Vec::new();

        for node_id in NodeId::targets(0..n) {
            if node_id == log_node_id {
                continue;
            }

            info!("{:?} // Connecting to node {:?}", log_node_id, node_id);

            let mut connection_results = node.node_connections().connect_to_node(node_id);

            connections.push((node_id, connection_results));
        }

        'outer: for (peer_id, conn_result) in connections {
            for conn in conn_result {
                match conn.await {
                    Ok(result) => {
                        if let Err(err) = result {
                            error!("{:?} // Failed to connect to {:?} for {:?}", log_node_id, peer_id, err);
                            continue 'outer;
                        }
                    }
                    Err(error) => {
                        error!("Failed to connect to the given node. {:?}", error);
                        continue 'outer;
                    }
                }
            }

            info!("{:?} // Established a new connection to node {:?}.", log_node_id, peer_id);
        }

        info!("{:?} // Connected to all other replicas.", log_node_id);

        info!("{:?} // Finished bootstrapping node.", log_node_id);

        let timeout_channel = channel::new_bounded_sync(1024);

        let mut replica = Self {
            // We start with the state transfer protocol to make sure everything is up to date
            replica_phase: ReplicaPhase::StateTransferProtocol,
            ordering_protocol,
            state_transfer_protocol,
            rq_pre_processor,
            timeouts,
            executor_handle: executor,
            node,
            execution_rx: exec_rx,
            execution_tx: exec_tx,
            processed_timeout: timeout_channel,
            persistent_log,
        };

        info!("{:?} // Requesting state", log_node_id);

        replica.state_transfer_protocol.request_latest_state(&mut replica.ordering_protocol)?;

        Ok(replica)
    }

    pub fn run(&mut self) -> Result<()> {
        let mut last_loop = Instant::now();

        loop {
            let now = Instant::now();

            self.receive_internal()?;

            metric_duration(REPLICA_INTERNAL_PROCESS_TIME_ID, now.elapsed());

            match self.replica_phase {
                ReplicaPhase::OrderingProtocol => {
                    let poll_res = self.ordering_protocol.poll();

                    trace!("{:?} // Polling ordering protocol with result {:?}", self.node.id(), poll_res);

                    match poll_res {
                        OrderProtocolPoll::RePoll => {
                            //Continue
                        }
                        OrderProtocolPoll::ReceiveFromReplicas => {
                            let start = Instant::now();

                            let network_message = self.node.node_incoming_rq_handling().receive_from_replicas(Some(REPLICA_WAIT_TIME)).unwrap();

                            metric_duration(REPLICA_TAKE_FROM_NETWORK_ID, start.elapsed());

                            let start = Instant::now();

                            if let Some(network_message) = network_message {
                                let (header, message) = network_message.into_inner();

                                let message = message.into_system();

                                match message {
                                    SystemMessage::ProtocolMessage(protocol) => {
                                        let start = Instant::now();

                                        match self.ordering_protocol.process_message(StoredMessage::new(header, protocol))? {
                                            OrderProtocolExecResult::Success => {
                                                //Continue execution
                                            }
                                            OrderProtocolExecResult::RunCst => {
                                                self.run_state_transfer_protocol()?;
                                            }
                                            OrderProtocolExecResult::Decided(decisions) => {
                                                self.execute_decisions(decisions)?;
                                            }
                                        }
                                    }
                                    SystemMessage::StateTransferMessage(state_transfer) => {
                                        self.state_transfer_protocol.handle_off_ctx_message(&mut self.ordering_protocol,
                                                                                            StoredMessage::new(header, state_transfer)).unwrap();
                                    }
                                    SystemMessage::ForwardedRequestMessage(fwd_reqs) => {
                                        // Send the forwarded requests to be handled, filtered and then passed onto the ordering protocol
                                        self.rq_pre_processor.send(PreProcessorMessage::ForwardedRequests(StoredMessage::new(header, fwd_reqs))).unwrap();
                                    }
                                    SystemMessage::ForwardedProtocolMessage(fwd_protocol) => {
                                        match self.ordering_protocol.process_message(fwd_protocol.into_inner())? {
                                            OrderProtocolExecResult::Success => {
                                                //Continue execution
                                            }
                                            OrderProtocolExecResult::RunCst => {
                                                self.run_state_transfer_protocol()?;
                                            }
                                            OrderProtocolExecResult::Decided(decisions) => {
                                                self.execute_decisions(decisions)?;
                                            }
                                        }
                                    }
                                    _ => {
                                        error!("{:?} // Received unsupported message {:?}", self.node.id(), message);
                                    }
                                }
                            } else {
                                // Receive timeouts in the beginning of the next iteration
                                continue;
                            }

                            metric_duration(ORDERING_PROTOCOL_PROCESS_TIME_ID, start.elapsed());
                            metric_increment(REPLICA_ORDERED_RQS_PROCESSED_ID, Some(1));
                        }
                        OrderProtocolPoll::Exec(message) => {
                            let start = Instant::now();

                            match self.ordering_protocol.process_message(message)? {
                                OrderProtocolExecResult::Success => {
                                    // Continue execution
                                }
                                OrderProtocolExecResult::RunCst => {
                                    self.run_state_transfer_protocol()?;
                                }
                                OrderProtocolExecResult::Decided(decided) => {
                                    self.execute_decisions(decided)?;
                                }
                            }

                            metric_duration(ORDERING_PROTOCOL_PROCESS_TIME_ID, start.elapsed());
                            metric_increment(REPLICA_ORDERED_RQS_PROCESSED_ID, Some(1));
                        }
                        OrderProtocolPoll::RunCst => {
                            self.run_state_transfer_protocol()?;
                        }
                        OrderProtocolPoll::Decided(decisions) => {
                            for decision in decisions {}
                        }
                    }
                }
                ReplicaPhase::StateTransferProtocol => {
                    let message = self.node.node_incoming_rq_handling().receive_from_replicas(Some(REPLICA_WAIT_TIME)).unwrap();

                    if let Some(message) = message {
                        let (header, message) = message.into_inner();

                        let message = message.into_system();

                        match message {
                            SystemMessage::ProtocolMessage(protocol) => {
                                self.ordering_protocol.handle_off_ctx_message(StoredMessage::new(header, protocol));
                            }
                            SystemMessage::StateTransferMessage(state_transfer) => {
                                let start = Instant::now();

                                let result = self.state_transfer_protocol.process_message(&mut self.ordering_protocol, StoredMessage::new(header, state_transfer))?;

                                match result {
                                    STResult::CstRunning => {}
                                    STResult::CstFinished(state, requests) => {
                                        info!("{:?} // State transfer finished. Installing state in executor and running ordering protocol", self.node.id());

                                        self.executor_handle.install_state(state, requests)?;

                                        self.run_ordering_protocol()?;
                                    }
                                    STResult::CstNotNeeded => {
                                        self.run_ordering_protocol()?;
                                    }
                                    STResult::RunCst => {
                                        self.run_state_transfer_protocol()?;
                                    }
                                }

                                metric_duration(STATE_TRANSFER_PROCESS_TIME_ID, start.elapsed());
                            }
                            _ => {}
                        }
                    }
                }
            }

            metric_duration(RUN_LATENCY_TIME_ID, last_loop.elapsed());

            last_loop = Instant::now();
        }

        Ok(())
    }

    fn execute_decisions(&mut self, decisions: Vec<ProtocolConsensusDecision<Request<S>>>) -> Result<()> {
        for decision in decisions {
            if let Some(decided) = decision.batch_info() {
                if let Err(err) = self.rq_pre_processor.send(PreProcessorMessage::DecidedBatch(decided.client_requests().clone())) {
                    error!("Error sending decided batch to pre processor: {:?}", err);
                }
            }

            if let Some(decision) = self.persistent_log.wait_for_batch_persistency_and_execute(decision)? {
                let (seq, batch, _) = decision.into();

                let last_seq_no_u32 = u32::from(seq);

                let checkpoint = if last_seq_no_u32 > 0 && last_seq_no_u32 % CHECKPOINT_PERIOD == 0 {
                    //We check that % == 0 so we don't start multiple checkpoints
                    self.state_transfer_protocol.handle_app_state_requested(seq)?
                } else {
                    ExecutionResult::Nil
                };

                match checkpoint {
                    ExecutionResult::Nil => {
                        self.executor_handle.queue_update(batch)?
                    }
                    ExecutionResult::BeginCheckpoint => {
                        self.executor_handle.queue_update_and_get_appstate(batch)?
                    }
                }
            }
        }

        Ok(())
    }

    /// FIXME: Do this with a select?
    fn receive_internal(&mut self) -> Result<()> {
        while let Ok(recvd) = self.execution_rx.try_recv() {
            match recvd {
                Message::ExecutionFinishedWithAppstate((seq, state)) => {
                    self.execution_finished_with_appstate(seq, state)?;
                }
                Message::Timeout(timeout) => {
                    self.timeout_received(timeout)?;
                    //info!("{:?} // Received and ignored timeout with {} timeouts {:?}", self.node.id(), timeout.len(), timeout);
                }
                Message::DigestedAppState(digested) => {
                    self.state_transfer_protocol.handle_state_received_from_app(&mut self.ordering_protocol, digested)?;
                }
                _ => {}
            }
        }

        while let Ok((timeouts, deleted)) = self.processed_timeout.1.try_recv() {
            self.processed_timeout_recvd(timeouts, deleted)?;
        }

        Ok(())
    }

    fn timeout_received(&mut self, timeouts: TimedOut) -> Result<()> {
        let start = Instant::now();

        let mut client_rq = Vec::with_capacity(timeouts.len());
        let mut cst_rq = Vec::with_capacity(timeouts.len());

        for timeout in timeouts {
            match timeout.timeout_kind() {
                TimeoutKind::ClientRequestTimeout(_) => {
                    client_rq.push(timeout);
                }
                TimeoutKind::Cst(_) => {
                    cst_rq.push(timeout);
                }
            }
        }

        if !client_rq.is_empty() {
            debug!("{:?} // Received client request timeouts: {}", self.node.id(), client_rq.len());

            self.rq_pre_processor.process_timeouts(client_rq, self.processed_timeout.0.clone());
        }

        if !cst_rq.is_empty() {
            debug!("{:?} // Received cst timeouts: {}", self.node.id(), cst_rq.len());

            match self.state_transfer_protocol.handle_timeout(&mut self.ordering_protocol, cst_rq)? {
                STTimeoutResult::RunCst => {
                    self.run_state_transfer_protocol()?;
                }
                _ => {}
            };
        }

        metric_duration(TIMEOUT_PROCESS_TIME_ID, start.elapsed());

        Ok(())
    }

    // Process a processed timeout request
    fn processed_timeout_recvd(&mut self, timed_out: Vec<RqTimeout>, to_delete: Vec<RqTimeout>) -> Result<()> {
        self.timeouts.cancel_client_rq_timeouts(Some(to_delete.into_iter().map(|t| match t.into_timeout_kind() {
            TimeoutKind::ClientRequestTimeout(info) => {
                info
            }
            _ => unreachable!()
        }).collect()));

        match self.ordering_protocol.handle_timeout(timed_out)? {
            OrderProtocolExecResult::RunCst => {
                self.run_state_transfer_protocol()?;
            }
            _ => {}
        };

        Ok(())
    }

    /// Run the ordering protocol on this replica
    fn run_ordering_protocol(&mut self) -> Result<()> {
        info!("{:?} // Running ordering protocol.", self.node.id());

        self.replica_phase = ReplicaPhase::OrderingProtocol;

        self.ordering_protocol.handle_execution_changed(true)?;

        Ok(())
    }

    /// Run the state transfer protocol on this replica
    fn run_state_transfer_protocol(&mut self) -> Result<()> {
        if self.replica_phase == ReplicaPhase::StateTransferProtocol {
            //TODO: This is here for when we receive various timeouts that consecutively call run cst
            // In reality, this should also lead to a new state request since the previous one could have
            // Timed out
            return Ok(());
        }

        info!("{:?} // Running state transfer protocol.", self.node.id());

        self.ordering_protocol.handle_execution_changed(false)?;

        // Start by requesting the current state from neighbour replicas
        self.state_transfer_protocol.request_latest_state(&mut self.ordering_protocol)?;

        self.replica_phase = ReplicaPhase::StateTransferProtocol;

        Ok(())
    }

    fn execution_finished_with_appstate(&mut self, seq: SeqNo, appstate: State<S>) -> Result<()> {
        let return_tx = self.execution_tx.clone();

        // Digest the app state before passing it on to the ordering protocols
        threadpool::execute(move || {
            let result = atlas_execution::serialize::digest_state::<S::Data>(&appstate);

            match result {
                Ok(digest) => {
                    let checkpoint = Checkpoint::new(seq, appstate, digest);

                    return_tx.send(Message::DigestedAppState(checkpoint)).unwrap();
                }
                Err(error) => {
                    error!("Failed to serialize and digest application state: {:?}", error)
                }
            }
        });

        Ok(())
    }
}

/// Checkpoint period.
///
/// Every `PERIOD` messages, the message log is cleared,
/// and a new log checkpoint is initiated.
/// TODO: Move this to an env variable as it can be highly dependent on the service implemented on top of it
pub const CHECKPOINT_PERIOD: u32 = 50000;
