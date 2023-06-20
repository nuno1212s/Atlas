//! Contains the server side core protocol logic of `febft`.

use std::marker::PhantomData;
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
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::{Node, NodeConnections, NodeIncomingRqHandler};
use atlas_communication::message::StoredMessage;
use atlas_execution::app::{Application, Request};
use atlas_execution::ExecutorHandle;
use atlas_core::messages::Message;
use atlas_core::messages::SystemMessage;
use atlas_core::ordering_protocol::{ExecutionResult, OrderingProtocol, OrderingProtocolArgs, ProtocolConsensusDecision};
use atlas_core::ordering_protocol::OrderProtocolExecResult;
use atlas_core::ordering_protocol::OrderProtocolPoll;
use atlas_core::persistent_log::{PersistableOrderProtocol, PersistableStateTransferProtocol, StatefulOrderingProtocolLog, WriteMode};
use atlas_core::request_pre_processing::{initialize_request_pre_processor, PreProcessorMessage, RequestPreProcessor};
use atlas_core::request_pre_processing::work_dividers::WDRoundRobin;
use atlas_core::serialize::{OrderingProtocolMessage, OrderProtocolLog, ServiceMsg, StateTransferMessage};
use atlas_core::state_transfer::{Checkpoint, StateTransferProtocol, STResult, STTimeoutResult};
use atlas_core::state_transfer::log_transfer::{LogTransferProtocol, LTResult, LTTimeoutResult, StatefulOrderProtocol};
use atlas_core::timeouts::{RqTimeout, TimedOut, Timeout, TimeoutKind, Timeouts};
use atlas_execution::serialize::ApplicationData;
use atlas_metrics::metrics::{metric_duration, metric_increment, metric_store_count};
use atlas_persistent_log::{NoPersistentLog, PersistentLog};
use crate::config::ReplicaConfig;
use crate::executable::{ReplicaReplier};
use crate::metric::{LOG_TRANSFER_PROCESS_TIME_ID, ORDERING_PROTOCOL_POLL_TIME_ID, ORDERING_PROTOCOL_PROCESS_TIME_ID, REPLICA_INTERNAL_PROCESS_TIME_ID, REPLICA_ORDERED_RQS_PROCESSED_ID, REPLICA_RQ_QUEUE_SIZE_ID, REPLICA_TAKE_FROM_NETWORK_ID, RUN_LATENCY_TIME_ID, STATE_TRANSFER_PROCESS_TIME_ID, TIMEOUT_PROCESS_TIME_ID};
use crate::persistent_log::SMRPersistentLog;
use crate::server::client_replier::Replier;

//pub mod observer;

pub mod client_replier;
pub mod follower_handling;
pub mod monolithic_server;
// pub mod rq_finalizer;

const REPLICA_MESSAGE_CHANNEL: usize = 1024;
pub const REPLICA_WAIT_TIME: Duration = Duration::from_millis(1000);

pub type StateTransferDone = Option<SeqNo>;
pub type LogTransferDone<D: ApplicationData> = Option<(SeqNo, SeqNo, Vec<D::Request>)>;

#[derive(Clone)]
pub(crate) enum ReplicaPhase<D> where D: ApplicationData {
    // The replica is currently executing the ordering protocol
    OrderingProtocol,
    // The replica is currently executing the state transfer protocol
    StateTransferProtocol {
        state_transfer: StateTransferDone,
        log_transfer: LogTransferDone<D>,
    },
}

pub struct Replica<S, D, OP, ST, LT, NT, PL> where D: ApplicationData + 'static,
                                                   OP: StatefulOrderProtocol<D, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + 'static,
                                                   LT: LogTransferProtocol<D, OP, NT, PL> + 'static,
                                                   ST: StateTransferProtocol<S, NT, PL> + PersistableStateTransferProtocol + 'static,
                                                   PL: SMRPersistentLog<D, OP::Serialization, OP::StateSerialization> + 'static, {
    replica_phase: ReplicaPhase<D>,
    // The ordering protocol, responsible for ordering requests
    ordering_protocol: OP,
    log_transfer_protocol: LT,
    rq_pre_processor: RequestPreProcessor<D::Request>,
    timeouts: Timeouts,
    executor_handle: ExecutorHandle<D>,
    // The networking layer for a Node in the network (either Client or Replica)
    node: Arc<NT>,
    // The handle to the execution and timeouts handler
    execution_rx: ChannelSyncRx<Message>,
    execution_tx: ChannelSyncTx<Message>,
    // THe handle for processed timeouts
    processed_timeout: (ChannelSyncTx<(Vec<RqTimeout>, Vec<RqTimeout>)>, ChannelSyncRx<(Vec<RqTimeout>, Vec<RqTimeout>)>),
    persistent_log: PL,

    st: PhantomData<(S, ST)>,
}

impl<S, D, OP, ST, LT, NT, PL> Replica<S, D, OP, ST, LT, NT, PL>
    where
        D: ApplicationData + 'static,
        OP: StatefulOrderProtocol<D, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + Send + 'static,
        LT: LogTransferProtocol<D, OP, NT, PL> + 'static,
        ST: StateTransferProtocol<S, NT, PL> + PersistableStateTransferProtocol + Send + 'static,
        NT: Node<ServiceMsg<D, OP::Serialization, ST::Serialization, LT::Serialization>> + 'static,
        PL: SMRPersistentLog<D, OP::Serialization, OP::StateSerialization> + 'static, {
    pub async fn bootstrap(cfg: ReplicaConfig<S, D, OP, ST, LT, NT, PL>, executor: ExecutorHandle<D>) -> Result<Self> {
        let ReplicaConfig {
            id: log_node_id,
            n,
            f,
            view,
            next_consensus_seq,
            db_path,
            op_config,
            lt_config,
            pl_config,
            node: node_config,
            p,
        } = cfg;

        debug!("{:?} // Bootstrapping replica, starting with networking", log_node_id);

        let node = NT::bootstrap(node_config).await?;

        debug!("{:?} // Initializing timeouts", log_node_id);

        let (exec_tx, exec_rx) = channel::new_bounded_sync(REPLICA_MESSAGE_CHANNEL);

        let (rq_pre_processor, batch_input) = initialize_request_pre_processor
            ::<WDRoundRobin, D, OP::Serialization, ST::Serialization, LT::Serialization, NT>(4, node.clone());

        let default_timeout = Duration::from_secs(3);

        // start timeouts handler
        let timeouts = Timeouts::new::<D>(log_node_id.clone(), Duration::from_millis(1),
                                          default_timeout, exec_tx.clone());

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

        let log_transfer_protocol = LT::initialize(lt_config, timeouts.clone(), node.clone(), persistent_log.clone())?;

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

        let state_transfer = ReplicaPhase::StateTransferProtocol {
            state_transfer: None,
            log_transfer: None,
        };

        let mut replica = Self {
// We start with the state transfer protocol to make sure everything is up to date
            replica_phase: state_transfer,
            ordering_protocol,
            log_transfer_protocol,
            rq_pre_processor,
            timeouts,
            executor_handle: executor,
            node,
            execution_rx: exec_rx,
            execution_tx: exec_tx,
            processed_timeout: timeout_channel,
            persistent_log,
            st: Default::default(),
        };

        info!("{:?} // Requesting state", log_node_id);

        replica.log_transfer_protocol.request_latest_log(&mut replica.ordering_protocol)?;

        Ok(replica)
    }

    pub fn run(&mut self, state_transfer: &mut ST) -> Result<()> {
        let now = Instant::now();

        self.receive_internal(state_transfer)?;

        metric_duration(REPLICA_INTERNAL_PROCESS_TIME_ID, now.elapsed());

        match &mut self.replica_phase {
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
                                            self.run_all_state_transfer(state_transfer)?;
                                        }
                                        OrderProtocolExecResult::Decided(decisions) => {
                                            self.execute_decisions(state_transfer, decisions)?;
                                        }
                                    }
                                }
                                SystemMessage::StateTransferMessage(state_transfer_msg) => {
                                    state_transfer.handle_off_ctx_message(self.ordering_protocol.view(), StoredMessage::new(header, state_transfer_msg))?;
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
                                            self.run_all_state_transfer(state_transfer)?;
                                        }
                                        OrderProtocolExecResult::Decided(decisions) => {
                                            self.execute_decisions(state_transfer, decisions)?;
                                        }
                                    }
                                }
                                SystemMessage::LogTransferMessage(log_transfer) => {
                                    self.log_transfer_protocol.handle_off_ctx_message(&mut self.ordering_protocol, StoredMessage::new(header, log_transfer)).unwrap();
                                }
                                _ => {
                                    error!("{:?} // Received unsupported message {:?}", self.node.id(), message);
                                }
                            }
                        } else {
                            // Receive timeouts in the beginning of the next iteration
                            return Ok(());
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
                                self.run_all_state_transfer(state_transfer)?;
                            }
                            OrderProtocolExecResult::Decided(decided) => {
                                self.execute_decisions(state_transfer, decided)?;
                            }
                        }

                        metric_duration(ORDERING_PROTOCOL_PROCESS_TIME_ID, start.elapsed());
                        metric_increment(REPLICA_ORDERED_RQS_PROCESSED_ID, Some(1));
                    }
                    OrderProtocolPoll::RunCst => {
                        self.run_state_transfer_protocol(state_transfer)?;
                    }
                    OrderProtocolPoll::Decided(decisions) => {
                        for decision in decisions {}
                    }
                }
            }
            ReplicaPhase::StateTransferProtocol { state_transfer: st_transfer_done, log_transfer: log_transfer_done } => {
                let message = self.node.node_incoming_rq_handling().receive_from_replicas(Some(REPLICA_WAIT_TIME)).unwrap();

                if let Some(message) = message {
                    let (header, message) = message.into_inner();

                    let message = message.into_system();

                    match message {
                        SystemMessage::ProtocolMessage(protocol) => {
                            self.ordering_protocol.handle_off_ctx_message(StoredMessage::new(header, protocol));
                        }
                        SystemMessage::StateTransferMessage(state_transfer_msg) => {
                            let start = Instant::now();

                            let result = state_transfer.process_message(self.ordering_protocol.view(), StoredMessage::new(header, state_transfer_msg))?;

                            match result {
                                STResult::StateTransferRunning => {}
                                STResult::StateTransferReady => {
                                    self.executor_handle.poll_state_channel()?;
                                }
                                STResult::StateTransferFinished(seq_no) => {
                                    info!("{:?} // State transfer finished. Installing state in executor and running ordering protocol", self.node.id());

                                    self.executor_handle.poll_state_channel()?;

                                    *st_transfer_done = Some(seq_no);
                                }
                                STResult::StateTransferNotNeeded(curr_seq) => {
                                    *st_transfer_done = Some(curr_seq);
                                }
                                STResult::RunStateTransfer => {
                                    self.run_state_transfer_protocol(state_transfer)?;
                                }
                            }

                            metric_duration(STATE_TRANSFER_PROCESS_TIME_ID, start.elapsed());
                        }
                        SystemMessage::LogTransferMessage(log_transfer) => {
                            let start = Instant::now();

                            let result = self.log_transfer_protocol.process_message(&mut self.ordering_protocol, StoredMessage::new(header, log_transfer))?;

                            match result {
                                LTResult::RunLTP => {
                                    self.run_log_transfer_protocol(state_transfer)?;
                                }
                                LTResult::Running => {}
                                LTResult::NotNeeded => {
                                    let log = self.ordering_protocol.current_log()?;

                                    *log_transfer_done = Some((log.first_seq().unwrap_or(SeqNo::ZERO), log.sequence_number(), Vec::new()));
                                }
                                LTResult::LTPFinished(first_seq, last_seq, requests_to_execute) => {
                                    info!("{:?} // State transfer finished. Installing state in executor and running ordering protocol", self.node.id());

                                    *log_transfer_done = Some((first_seq, last_seq, requests_to_execute));
                                }
                            }

                            metric_duration(LOG_TRANSFER_PROCESS_TIME_ID, start.elapsed());
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    fn execute_decisions(&mut self, state_transfer: &mut ST, decisions: Vec<ProtocolConsensusDecision<D::Request>>) -> Result<()> {
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
                    state_transfer.handle_app_state_requested(self.ordering_protocol.view(), seq)?
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
    fn receive_internal(&mut self, state_transfer: &mut ST) -> Result<()> {
        while let Ok(recvd) = self.execution_rx.try_recv() {
            match recvd {
                Message::Timeout(timeout) => {
                    self.timeout_received(state_transfer, timeout)?;
                    //info!("{:?} // Received and ignored timeout with {} timeouts {:?}", self.node.id(), timeout.len(), timeout);
                }
                _ => {}
            }
        }

        while let Ok((timeouts, deleted)) = self.processed_timeout.1.try_recv() {
            self.processed_timeout_recvd(state_transfer, timeouts, deleted)?;
        }

        Ok(())
    }

    fn timeout_received(&mut self, state_transfer: &mut ST, timeouts: TimedOut) -> Result<()> {
        let start = Instant::now();

        let mut client_rq = Vec::with_capacity(timeouts.len());
        let mut cst_rq = Vec::new();
        let mut log_transfer = Vec::new();

        for timeout in timeouts {
            match timeout.timeout_kind() {
                TimeoutKind::ClientRequestTimeout(_) => {
                    client_rq.push(timeout);
                }
                TimeoutKind::Cst(_) => {
                    cst_rq.push(timeout);
                }
                TimeoutKind::LogTransfer(_) => {
                    log_transfer.push(timeout);
                }
            }
        }

        if !client_rq.is_empty() {
            debug!("{:?} // Received client request timeouts: {}", self.node.id(), client_rq.len());

            self.rq_pre_processor.process_timeouts(client_rq, self.processed_timeout.0.clone());
        }

        if !cst_rq.is_empty() {
            debug!("{:?} // Received cst timeouts: {}", self.node.id(), cst_rq.len());

            match state_transfer.handle_timeout(self.ordering_protocol.view(), cst_rq)? {
                STTimeoutResult::RunCst => {
                    self.run_state_transfer_protocol(state_transfer)?;
                }
                _ => {}
            };
        }

        if !log_transfer.is_empty() {
            debug!("{:?} // Received log transfer timeouts: {}", self.node.id(), log_transfer.len());

            match self.log_transfer_protocol.handle_timeout(log_transfer)? {
                LTTimeoutResult::RunLTP => {
                    self.run_log_transfer_protocol(state_transfer)?;
                }
                _ => {}
            };
        }

        metric_duration(TIMEOUT_PROCESS_TIME_ID, start.elapsed());

        Ok(())
    }

    // Process a processed timeout request
    fn processed_timeout_recvd(&mut self, state_transfer: &mut ST, timed_out: Vec<RqTimeout>, to_delete: Vec<RqTimeout>) -> Result<()> {
        self.timeouts.cancel_client_rq_timeouts(Some(to_delete.into_iter().map(|t| match t.into_timeout_kind() {
            TimeoutKind::ClientRequestTimeout(info) => {
                info
            }
            _ => unreachable!()
        }).collect()));

        match self.ordering_protocol.handle_timeout(timed_out)? {
            OrderProtocolExecResult::RunCst => {
                self.run_state_transfer_protocol(state_transfer)?;
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

    fn run_log_transfer_protocol(&mut self, state_transfer: &mut ST) -> Result<()> {
        info!("{:?} // Running log transfer protocol.", self.node.id());

        match &mut self.replica_phase {
            ReplicaPhase::OrderingProtocol => {
                self.run_all_state_transfer(state_transfer)?;
            }
            ReplicaPhase::StateTransferProtocol { log_transfer, .. } => {
                *log_transfer = None;

                self.log_transfer_protocol.request_latest_log(&mut self.ordering_protocol)?;
            }
        }

        Ok(())
    }

    fn run_all_state_transfer(&mut self, state_transfer: &mut ST) -> Result<()> {
        info!("{:?} // Running all state transfer.", self.node.id());

        match &mut self.replica_phase {
            ReplicaPhase::OrderingProtocol => {
                self.ordering_protocol.handle_execution_changed(false)?;

                self.replica_phase = ReplicaPhase::StateTransferProtocol {
                    state_transfer: None,
                    log_transfer: None,
                };
            }
            ReplicaPhase::StateTransferProtocol { state_transfer, log_transfer } => {
                *state_transfer = None;
                *log_transfer = None;
            }
        }

        // Start by requesting the current state from neighbour replicas
        state_transfer.request_latest_state(self.ordering_protocol.view())?;
        self.log_transfer_protocol.request_latest_log(&mut self.ordering_protocol)?;

        Ok(())
    }

    /// Run the state transfer protocol on this replica
    fn run_state_transfer_protocol(&mut self, state_transfer_p: &mut ST) -> Result<()> {
        info!("{:?} // Running log transfer protocol.", self.node.id());

        match &mut self.replica_phase {
            ReplicaPhase::OrderingProtocol => {
                self.run_all_state_transfer(state_transfer_p)?;
            }
            ReplicaPhase::StateTransferProtocol { state_transfer, .. } => {
                *state_transfer = None;

                state_transfer_p.request_latest_state(self.ordering_protocol.view())?;
            }
        }

        Ok(())
    }
}

/// Checkpoint period.
///
/// Every `PERIOD` messages, the message log is cleared,
/// and a new log checkpoint is initiated.
/// TODO: Move this to an env variable as it can be highly dependent on the service implemented on top of it
pub const CHECKPOINT_PERIOD: u32 = 50000;

impl<D> PartialEq for ReplicaPhase<D> where D: ApplicationData {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ReplicaPhase::OrderingProtocol, ReplicaPhase::OrderingProtocol) => true,
            (ReplicaPhase::StateTransferProtocol { .. }, ReplicaPhase::StateTransferProtocol { .. }) => true,
            (_, _) => false
        }
    }
}