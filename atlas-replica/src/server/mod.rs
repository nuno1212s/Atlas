//! Contains the server side core protocol logic of `febft`.

use std::fmt::{Debug, Formatter, write};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{debug, error, info, trace, warn};

use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::StoredMessage;
use atlas_communication::protocol_node::{NodeIncomingRqHandler, ProtocolNetworkNode};
use atlas_communication::NetworkNode;
use atlas_core::log_transfer::{LogTransferProtocol, LTResult, LTTimeoutResult};
use atlas_core::messages::Message;
use atlas_core::messages::SystemMessage;
use atlas_core::ordering_protocol::{ExecutionResult, OrderingProtocolArgs, ProtocolConsensusDecision};
use atlas_core::ordering_protocol::networking::serialize::OrderProtocolLog;
use atlas_core::ordering_protocol::OrderProtocolExecResult;
use atlas_core::ordering_protocol::OrderProtocolPoll;
use atlas_core::ordering_protocol::reconfigurable_order_protocol::{ReconfigurableOrderProtocol, ReconfigurationAttemptResult};
use atlas_core::ordering_protocol::stateful_order_protocol::StatefulOrderProtocol;
use atlas_core::persistent_log::OperationMode;
use atlas_core::persistent_log::PersistableOrderProtocol;
use atlas_core::persistent_log::PersistableStateTransferProtocol;
use atlas_core::reconfiguration_protocol::{AlterationFailReason, QuorumAlterationResponse, QuorumAttemptJoinResponse, QuorumReconfigurationMessage, QuorumReconfigurationResponse, ReconfigurableNodeTypes, ReconfigurationProtocol};
use atlas_core::request_pre_processing::{initialize_request_pre_processor, PreProcessorMessage, RequestPreProcessor};
use atlas_core::request_pre_processing::work_dividers::WDRoundRobin;
use atlas_core::smr::networking::SMRNetworkNode;
use atlas_core::state_transfer::{StateTransferProtocol, STResult, STTimeoutResult};
use atlas_core::timeouts::{RqTimeout, TimedOut, TimeoutKind, Timeouts};
use atlas_execution::ExecutorHandle;
use atlas_execution::serialize::ApplicationData;
use atlas_metrics::metrics::{metric_duration, metric_increment};
use atlas_persistent_log::NoPersistentLog;

use crate::config::ReplicaConfig;
use crate::metric::{LOG_TRANSFER_PROCESS_TIME_ID, ORDERING_PROTOCOL_PROCESS_TIME_ID, REPLICA_INTERNAL_PROCESS_TIME_ID, REPLICA_ORDERED_RQS_PROCESSED_ID, REPLICA_TAKE_FROM_NETWORK_ID, STATE_TRANSFER_PROCESS_TIME_ID, TIMEOUT_PROCESS_TIME_ID};
use crate::persistent_log::SMRPersistentLog;


pub mod client_replier;
pub mod follower_handling;
pub mod monolithic_server;
mod divisible_state_server;
// pub mod rq_finalizer;

const REPLICA_MESSAGE_CHANNEL: usize = 1024;
pub const REPLICA_WAIT_TIME: Duration = Duration::from_millis(1000);

pub type StateTransferDone = Option<SeqNo>;
pub type LogTransferDone<R> = Option<(SeqNo, SeqNo, Vec<R>)>;

#[derive(Clone)]
pub(crate) enum ReplicaPhase<R> {
    // The replica is currently executing the ordering protocol
    OrderingProtocol,
    // The replica is currently executing the state transfer protocol
    StateTransferProtocol {
        state_transfer: StateTransferDone,
        log_transfer: LogTransferDone<R>,
    },
}

pub struct Replica<RP, S, D, OP, ST, LT, NT, PL> where D: ApplicationData + 'static,
                                                       OP: StatefulOrderProtocol<D, NT, PL> + PersistableOrderProtocol<D, OP::Serialization, OP::StateSerialization> + 'static,
                                                       LT: LogTransferProtocol<D, OP, NT, PL> + 'static,
                                                       ST: StateTransferProtocol<S, NT, PL> + PersistableStateTransferProtocol + 'static,
                                                       PL: SMRPersistentLog<D, OP::Serialization, OP::StateSerialization, OP::PermissionedSerialization> + 'static,
                                                       RP: ReconfigurationProtocol + 'static {
    replica_phase: ReplicaPhase<D::Request>,

    quorum_reconfig_data: QuorumReconfig,

    // The ordering protocol, responsible for ordering requests
    ordering_protocol: OP,
    log_transfer_protocol: LT,
    rq_pre_processor: RequestPreProcessor<D::Request>,
    timeouts: Timeouts,
    executor_handle: ExecutorHandle<D>,
    // The networking layer for a Node in the network (either Client or Replica)
    node: Arc<NT>,
    // The handle to the execution and timeouts handler
    execution: (ChannelSyncRx<Message>, ChannelSyncTx<Message>),
    // THe handle for processed timeouts
    processed_timeout: (ChannelSyncTx<(Vec<RqTimeout>, Vec<RqTimeout>)>, ChannelSyncRx<(Vec<RqTimeout>, Vec<RqTimeout>)>),

    // Receive reconfiguration messages from the reconfiguration protocol
    reconf_receive: ChannelSyncRx<QuorumReconfigurationMessage>,
    // reconfiguration protocol send
    reconf_tx: ChannelSyncTx<QuorumReconfigurationResponse>,

    persistent_log: PL,
    // The reconfiguration protocol handle
    reconfig_protocol: RP,

    st: PhantomData<(S, ST)>,
}

/// This is used to keep track of the node that is currently
/// attempting to join the server
pub struct QuorumReconfig {
    node_pending_join: Option<NodeId>,
}

impl<RP, S, D, OP, ST, LT, NT, PL> Replica<RP, S, D, OP, ST, LT, NT, PL>
    where
        RP: ReconfigurationProtocol + 'static,
        D: ApplicationData + 'static,
        OP: StatefulOrderProtocol<D, NT, PL> + PersistableOrderProtocol<D, OP::Serialization, OP::StateSerialization> + ReconfigurableOrderProtocol<RP::Serialization> + Send + 'static,
        LT: LogTransferProtocol<D, OP, NT, PL> + 'static,
        ST: StateTransferProtocol<S, NT, PL> + PersistableStateTransferProtocol + Send + 'static,
        NT: SMRNetworkNode<RP::InformationProvider, RP::Serialization, D, OP::Serialization, ST::Serialization, LT::Serialization> + 'static,
        PL: SMRPersistentLog<D, OP::Serialization, OP::StateSerialization, OP::PermissionedSerialization> + 'static, {
    async fn bootstrap(cfg: ReplicaConfig<RP, S, D, OP, ST, LT, NT, PL>, executor: ExecutorHandle<D>) -> Result<Self> {
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
            reconfig_node, p,
        } = cfg;

        debug!("{:?} // Bootstrapping replica, starting with networking", log_node_id);

        let network_info = RP::init_default_information(reconfig_node)?;

        let node = Arc::new(NT::bootstrap(network_info.clone(), node_config).await?);

        let (reconf_tx, reconf_rx) = channel::new_bounded_sync(REPLICA_MESSAGE_CHANNEL);
        let (reconf_response_tx, reply_rx) = channel::new_bounded_sync(REPLICA_MESSAGE_CHANNEL);

        let default_timeout = Duration::from_secs(3);

        let (exec_tx, exec_rx) = channel::new_bounded_sync(REPLICA_MESSAGE_CHANNEL);

        debug!("{:?} // Initializing timeouts", log_node_id);
        // start timeouts handler
        let timeouts = Timeouts::new::<D>(log_node_id.clone(), Duration::from_millis(1),
                                          default_timeout, exec_tx.clone());

        let replica_node_args = ReconfigurableNodeTypes::QuorumNode(reconf_tx, reply_rx);

        debug!("{:?} // Initializing reconfiguration protocol", log_node_id);
        let reconfig_protocol = RP::initialize_protocol(network_info, node.clone(), timeouts.clone(), replica_node_args, OP::get_n_for_f(1)).await?;

        info!("{:?} // Waiting for reconfiguration protocol to stabilize", log_node_id);

        let mut quorum = loop {
            let message = reconf_rx.recv().unwrap();

            match message {
                QuorumReconfigurationMessage::ReconfigurationProtocolStable(quorum) => {
                    reconf_response_tx.send(QuorumReconfigurationResponse::QuorumStableResponse(true)).unwrap();

                    break quorum;
                }
                _ => {
                    error!("Received request for quorum view alteration, but we are not even done with reconfiguration stabilization?");

                    return Err(Error::simple_with_msg(ErrorKind::ReconfigurationNotStable, "Received alteration request before stable quorum was reached"));
                }
            }
        };

        if quorum.len() < OP::get_n_for_f(1) {
            error!("Received a stable message, but the quorum is not big enough?");

            return Err(Error::simple_with_msg(ErrorKind::ReconfigurationNotStable, "The reconfiguration protocol is not stable with , but we received a stable message?"));
        }

        info!("{:?} // Reconfiguration protocol stabilized with {} nodes ({:?}), starting replica", log_node_id, quorum.len(), quorum);

        let (rq_pre_processor, batch_input) = initialize_request_pre_processor
            ::<WDRoundRobin, D, OP::Serialization, ST::Serialization, LT::Serialization, NT>(4, node.clone());

        let persistent_log = PL::init_log::<String, NoPersistentLog, OP, ST>(executor.clone(), db_path)?;

        let log = persistent_log.read_state(OperationMode::BlockingSync)?;

        let op_args = OrderingProtocolArgs(executor.clone(), timeouts.clone(),
                                           rq_pre_processor.clone(),
                                           batch_input, node.clone(),
                                           persistent_log.clone(), quorum);

        let ordering_protocol = if let Some((view, log)) = log {
            // Initialize the ordering protocol
            OP::initialize_with_initial_state(op_config, op_args, log)?
        } else {
            OP::initialize(op_config, op_args)?
        };

        let log_transfer_protocol = LT::initialize(lt_config, timeouts.clone(), node.clone(), persistent_log.clone())?;

        info!("{:?} // Finished bootstrapping node.", log_node_id);

        let timeout_channel = channel::new_bounded_sync(1024);

        let state_transfer = ReplicaPhase::StateTransferProtocol {
            state_transfer: None,
            log_transfer: None,
        };

        let mut replica = Self {
// We start with the state transfer protocol to make sure everything is up to date
            replica_phase: state_transfer,
            quorum_reconfig_data: QuorumReconfig { node_pending_join: None },
            ordering_protocol,
            log_transfer_protocol,
            rq_pre_processor,
            timeouts,
            executor_handle: executor,
            node,
            execution: (exec_rx, exec_tx),
            processed_timeout: timeout_channel,
            reconf_receive: reconf_rx,
            reconf_tx: reconf_response_tx,
            persistent_log,
            reconfig_protocol,
            st: Default::default(),
        };

        info!("{:?} // Requesting state", log_node_id);

        replica.log_transfer_protocol.request_latest_log(&mut replica.ordering_protocol)?;

        Ok(replica)
    }

    fn id(&self) -> NodeId {
        NetworkNode::id(&*self.node)
    }

    pub fn run(&mut self, state_transfer: &mut ST) -> Result<()> {
        let now = Instant::now();

        self.receive_internal(state_transfer)?;

        metric_duration(REPLICA_INTERNAL_PROCESS_TIME_ID, now.elapsed());

        match &self.replica_phase {
            ReplicaPhase::OrderingProtocol => {
                let poll_res = self.ordering_protocol.poll();

                trace!("{:?} // Polling ordering protocol with result {:?}", NetworkNode::id(&*self.node), poll_res);

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
                                        OrderProtocolExecResult::QuorumJoined(decision, node_id, current_quorum) => {
                                            if let Some(decision) = decision {
                                                self.execute_decisions(state_transfer, decision)?;
                                            }

                                            self.handle_quorum_joined(node_id, current_quorum, state_transfer)?;
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
                                        OrderProtocolExecResult::QuorumJoined(decision, node_id, current_quorum) => {
                                            if let Some(decision) = decision {
                                                self.execute_decisions(state_transfer, decision)?;
                                            }

                                            self.handle_quorum_joined(node_id, current_quorum, state_transfer)?;
                                        }
                                    }
                                }
                                SystemMessage::LogTransferMessage(log_transfer) => {
                                    self.log_transfer_protocol.handle_off_ctx_message(&mut self.ordering_protocol, StoredMessage::new(header, log_transfer)).unwrap();
                                }
                                _ => {
                                    error!("{:?} // Received unsupported message {:?}", NetworkNode::id(&*self.node), message);
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
                            OrderProtocolExecResult::QuorumJoined(decision, node_id, current_quorum) => {
                                if let Some(decision) = decision {
                                    self.execute_decisions(state_transfer, decision)?;
                                }

                                self.handle_quorum_joined(node_id, current_quorum, state_transfer)?;
                            }
                        }
                        metric_duration(ORDERING_PROTOCOL_PROCESS_TIME_ID, start.elapsed());
                        metric_increment(REPLICA_ORDERED_RQS_PROCESSED_ID, Some(1));
                    }
                    OrderProtocolPoll::RunCst => {
                        self.run_all_state_transfer(state_transfer)?;
                    }
                    OrderProtocolPoll::Decided(decisions) => {
                        self.execute_decisions(state_transfer, decisions)?;
                    }
                    OrderProtocolPoll::QuorumJoined(decision, node_id, current_quorum) => {
                        if let Some(decision) = decision {
                            self.execute_decisions(state_transfer, decision)?;
                        }

                        self.handle_quorum_joined(node_id, current_quorum, state_transfer)?;
                    }
                }
            }
            ReplicaPhase::StateTransferProtocol { state_transfer: st_transfer_done, log_transfer: log_transfer_done } => {
                let message = self.node.node_incoming_rq_handling().receive_from_replicas(Some(REPLICA_WAIT_TIME)).unwrap();

                if let Some(message) = message {
                    let (header, message) = message.into_inner();

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
                                    info!("{:?} // State transfer finished. Installing state in executor and running ordering protocol", NetworkNode::id(&*self.node));

                                    self.executor_handle.poll_state_channel()?;

                                    self.state_transfer_protocol_done(state_transfer, seq_no)?;
                                }
                                STResult::StateTransferNotNeeded(curr_seq) => {
                                    self.state_transfer_protocol_done(state_transfer, curr_seq)?;
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

                                    self.log_transfer_protocol_done(state_transfer, log.first_seq().unwrap_or(SeqNo::ZERO), log.sequence_number(), Vec::new())?;
                                }
                                LTResult::LTPFinished(first_seq, last_seq, requests_to_execute) => {
                                    info!("{:?} // State transfer finished. Installing state in executor and running ordering protocol", NetworkNode::id(&*self.node));
                                    self.log_transfer_protocol_done(state_transfer, first_seq, last_seq, requests_to_execute)?;
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
        while let Ok(recvd) = self.execution.0.try_recv() {
            match recvd {
                Message::Timeout(timeout) => {
                    self.timeout_received(state_transfer, timeout)?;
//info!("{:?} // Received and ignored timeout with {} timeouts {:?}", self.node.id(), timeout.len(), timeout);
                }
                _ => {}
            }
        }

        while let Ok(received) = self.reconf_receive.try_recv() {
            match received {
                QuorumReconfigurationMessage::RequestQuorumJoin(node) => {
                    info!("Received request for quorum view alteration for {:?}, current phase: {:?}", node, self.replica_phase);

                    self.attempt_quorum_join(node)?;
                }
                QuorumReconfigurationMessage::AttemptToJoinQuorum => {
                    info!("Received request to attempt to join quorum, current phase: {:?}", self.replica_phase);

                    self.attempt_to_join_quorum()?;
                }
                QuorumReconfigurationMessage::QuorumUpdated(new_quorum) => {
                    info!("Received quorum updated message, dealing with it");

                    // If we receive a quorum updated message, that means that we are not a part of the quorum,
                    // and there is probably new information that we need to know about from the ordering protocol.
                    //TODO: Here we want to check if we are currently attempting to join and if we are then take appropriate actions
                    if new_quorum.contains(&self.id()) {
                        unreachable!("We are a part of the quorum and we have received a quorum updated message? This information should come from the ordering protocol instead");
                    } else {
                        // We are not a part of the quorum, so we need to start the state transfer protocol
                        // In order to receive any new information about the ordering protocol status (Like new views)

                        error!("TODO: Handle quorum updated")
                    }
                }
                QuorumReconfigurationMessage::ReconfigurationProtocolStable(_) => {
                    info!("Received reconfiguration protocol stable but we are already done?");
                }
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
        let mut reconfiguration = Vec::new();

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
                TimeoutKind::Reconfiguration(_) => {
                    reconfiguration.push(timeout);
                }
            }
        }

        if !client_rq.is_empty() {
            debug!("{:?} // Received client request timeouts: {}", NetworkNode::id(&*self.node), client_rq.len());

            self.rq_pre_processor.process_timeouts(client_rq, self.processed_timeout.0.clone());
        }

        if !cst_rq.is_empty() {
            debug!("{:?} // Received cst timeouts: {}", NetworkNode::id(&*self.node), cst_rq.len());

            match state_transfer.handle_timeout(self.ordering_protocol.view(), cst_rq)? {
                STTimeoutResult::RunCst => {
                    self.run_state_transfer_protocol(state_transfer)?;
                }
                _ => {}
            };
        }

        if !log_transfer.is_empty() {
            debug!("{:?} // Received log transfer timeouts: {}", NetworkNode::id(&*self.node), log_transfer.len());

            match self.log_transfer_protocol.handle_timeout(log_transfer)? {
                LTTimeoutResult::RunLTP => {
                    self.run_log_transfer_protocol(state_transfer)?;
                }
                _ => {}
            };
        }

        if !reconfiguration.is_empty() {
            debug!("{:?} // Received reconfiguration timeouts: {}", NetworkNode::id(&*self.node), reconfiguration.len());

            self.reconfig_protocol.handle_timeout(reconfiguration)?;
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
                self.run_all_state_transfer(state_transfer)?;
            }
            _ => {}
        };

        Ok(())
    }

    /// Run the ordering protocol on this replica
    fn run_ordering_protocol(&mut self) -> Result<()> {
        info!("{:?} // Running ordering protocol.", NetworkNode::id(&*self.node));

        let phase = std::mem::replace(&mut self.replica_phase, ReplicaPhase::OrderingProtocol);

        match phase {
            ReplicaPhase::OrderingProtocol => {}
            ReplicaPhase::StateTransferProtocol { log_transfer, state_transfer } => {
                if let Some((log_first, log_last, requests_to_execute)) = log_transfer {
                    /// deliver the requests to the executor
                    self.executor_handle.catch_up_to_quorum(requests_to_execute)?;
                }
            }
        }

        self.replica_phase = ReplicaPhase::OrderingProtocol;

        self.ordering_protocol.handle_execution_changed(true)?;

        if let Some(node) = self.quorum_reconfig_data.pop_pending_node_join() {
            self.attempt_quorum_join(node)?;
        }

        Ok(())
    }

    /// Is the log transfer protocol finished?
    fn is_log_transfer_done(log: &LogTransferDone<D::Request>) -> bool {
        return log.is_some();
    }

    /// Is the state transfer protocol finished?
    fn is_state_transfer_done(state: &StateTransferDone) -> bool {
        return state.is_some();
    }

    /// Mark the log transfer protocol as done
    fn log_transfer_protocol_done(&mut self, state_transfer_protocol: &mut ST, first_seq: SeqNo, last_seq: SeqNo, requests_to_execute: Vec<D::Request>) -> Result<()> {
        let log_transfer_protocol_done = match &mut self.replica_phase {
            ReplicaPhase::OrderingProtocol => false,
            ReplicaPhase::StateTransferProtocol { log_transfer, state_transfer } => {
                *log_transfer = Some((first_seq, last_seq, requests_to_execute));

                if Self::is_log_transfer_done(log_transfer) & &Self::is_state_transfer_done(state_transfer) {
                    true
                } else {
                    false
                }
            }
        };

        if log_transfer_protocol_done {
            self.finish_state_transfer(state_transfer_protocol)?;
        }

        Ok(())
    }

    /// Mark the state transfer protocol as done
    fn state_transfer_protocol_done(&mut self, state_transfer_protocol: &mut ST, seq_no: SeqNo) -> Result<()> {
        let state_transfer_protocol_done = match &mut self.replica_phase {
            ReplicaPhase::OrderingProtocol => false,
            ReplicaPhase::StateTransferProtocol { state_transfer, log_transfer } => {
                *state_transfer = Some(seq_no);

                if Self::is_log_transfer_done(log_transfer) & &Self::is_state_transfer_done(state_transfer) {
                    true
                } else {
                    false
                }
            }
        };

        if state_transfer_protocol_done {
            self.finish_state_transfer(state_transfer_protocol)?;
        }

        Ok(())
    }

    /// Handle the log transfer and the state transfer protocol finishing
    /// their execution
    fn finish_state_transfer(&mut self, state_transfer_protocol: &mut ST) -> Result<()> {
        match &self.replica_phase {
            ReplicaPhase::OrderingProtocol => {}
            ReplicaPhase::StateTransferProtocol { state_transfer, log_transfer } => {
                let state_transfer = state_transfer.clone().unwrap();
                let (log_first, log_last, _) = log_transfer.as_ref().unwrap();

// If both the state and the log start at 0, then we can just run the ordering protocol since
// There is no state currently present.
                if state_transfer.next() != *log_first && (state_transfer != SeqNo::ZERO && *log_first != SeqNo::ZERO) {
                    error!("{:?} // Log transfer protocol and state transfer protocol are not in sync. Received {:?} state and {:?} - {:?} log", self.id(), state_transfer, * log_first, * log_last);

// Run both the protocols again
// This might work better since we already have a more up-to-date state (in
// The case of a hugely large state) so the state transfer protocol should take less time
                    self.run_all_state_transfer(state_transfer_protocol)?;
                } else {
                    info!("{:?} // State transfer protocol and log transfer protocol are in sync. Received {:?} state and {:?} - {:?} log", self.id(), state_transfer, * log_first, * log_last);

                    /// If the protocols are lined up so we can start running the ordering protocol
                    self.run_ordering_protocol()?;
                }
            }
        }

        Ok(())
    }

    /// Run the state transfer and log transfer protocols
    fn run_all_state_transfer(&mut self, state_transfer: &mut ST) -> Result<()> {
        info!("{:?} // Running state and log transfer protocols. {:?}", NetworkNode::id(&*self.node), self.replica_phase);

        match &mut self.replica_phase {
            ReplicaPhase::OrderingProtocol => {
                self.ordering_protocol.handle_execution_changed(false)?;

                self.replica_phase = ReplicaPhase::StateTransferProtocol {
                    state_transfer: None,
                    log_transfer: None,
                };
            }
            ReplicaPhase::StateTransferProtocol { state_transfer, log_transfer } => {
                warn!("{:?} // Why would we want to run the protocols when we are already running them?", NetworkNode::id(&*self.node));

                return Ok(());
            }
        }

// Start by requesting the current state from neighbour replicas
        state_transfer.request_latest_state(self.ordering_protocol.view())?;
        self.log_transfer_protocol.request_latest_log(&mut self.ordering_protocol)?;

        Ok(())
    }

    /// Run the state transfer protocol on this replica
    fn run_state_transfer_protocol(&mut self, state_transfer_p: &mut ST) -> Result<()> {
        info!("{:?} // Running state transfer protocol. {:?}", NetworkNode::id(&*self.node), self.replica_phase);

        match &mut self.replica_phase {
            ReplicaPhase::OrderingProtocol => {
                self.run_all_state_transfer(state_transfer_p)?;
            }
            ReplicaPhase::StateTransferProtocol { state_transfer, .. } => {
                warn!("{:?} // Why would we want to run the state transfer protocol when we are already running it?", NetworkNode::id(&*self.node));

                *state_transfer = None;

                state_transfer_p.request_latest_state(self.ordering_protocol.view())?;
            }
        }

        Ok(())
    }

    /// Runs the log transfer protocol on this replica
    fn run_log_transfer_protocol(&mut self, state_transfer: &mut ST) -> Result<()> {
        info!("{:?} // Running log transfer protocol. {:?}", NetworkNode::id(&*self.node), self.replica_phase);

        match &mut self.replica_phase {
            ReplicaPhase::OrderingProtocol => {
                self.run_all_state_transfer(state_transfer)?;
            }
            ReplicaPhase::StateTransferProtocol { log_transfer, .. } => {
                warn!("{:?} // Why would we want to run the log transfer protocol when we are already running it?", NetworkNode::id(&*self.node));

                *log_transfer = None;

                self.log_transfer_protocol.request_latest_log(&mut self.ordering_protocol)?;
            }
        }

        Ok(())
    }

    fn attempt_to_join_quorum(&mut self) -> Result<()> {
        match self.ordering_protocol.joining_quorum()? {
            ReconfigurationAttemptResult::Failed => {
                self.reply_to_attempt_quorum_join(true)?;
            }
            ReconfigurationAttemptResult::CurrentlyReconfiguring(_) => {
                self.reply_to_attempt_quorum_join(true)?;
            }
            ReconfigurationAttemptResult::InProgress => {}
            ReconfigurationAttemptResult::Successful | ReconfigurationAttemptResult::AlreadyPartOfQuorum => {
                // If we are already part of the quorum, then we can immediately reply to it
                self.reply_to_attempt_quorum_join(false)?;
            }
        };

        Ok(())
    }

    /// Attempt to join the quorum
    fn attempt_quorum_join(&mut self, node: NodeId) -> Result<()> {
        match self.replica_phase {
            ReplicaPhase::OrderingProtocol => {
                let quorum_node_join = self.ordering_protocol.attempt_quorum_node_join(node)?;

                debug!("{:?} // Attempting to join quorum with result {:?}", self.id(), quorum_node_join);

                match quorum_node_join {
                    ReconfigurationAttemptResult::Failed => {
                        self.reply_to_quorum_entrance_request(node, Some(AlterationFailReason::Failed))?;
                    }
                    ReconfigurationAttemptResult::AlreadyPartOfQuorum => {
                        self.reply_to_quorum_entrance_request(node, Some(AlterationFailReason::AlreadyPartOfQuorum))?;
                    }
                    ReconfigurationAttemptResult::CurrentlyReconfiguring(_) => {
                        self.reply_to_quorum_entrance_request(node, Some(AlterationFailReason::OngoingReconfiguration))?;
                    }
                    ReconfigurationAttemptResult::InProgress => {}
                    ReconfigurationAttemptResult::Successful => {
                        self.reply_to_quorum_entrance_request(node, None)?;
                    }
                };
            }
            ReplicaPhase::StateTransferProtocol { .. } => {
                self.append_pending_join_node(node);
            }
        }

        Ok(())
    }

    /// We are pending a node join
    fn append_pending_join_node(&mut self, node: NodeId) -> bool {
        self.quorum_reconfig_data.append_pending_node_join(node)
    }

    fn handle_quorum_joined(&mut self, node: NodeId, members: Vec<NodeId>, state_transfer: &mut ST) -> Result<()> {
        if node == self.id() {
            info!("{:?} // We have joined the quorum, responding to the reconfiguration protocol", self.id());

            self.reply_to_attempt_quorum_join(false)?;

            return Ok(());
        }

        self.reply_to_quorum_entrance_request(node, None)?;

        Ok(())
    }

    fn reply_to_attempt_quorum_join(&mut self, failed: bool) -> Result<()> {
        info!("{:?} // Sending attempt quorum join response to reconfiguration protocol. Has failed: {:?}", self.id(), failed);

        if failed {
            self.reconf_tx.send(QuorumReconfigurationResponse::QuorumAttemptJoinResponse(QuorumAttemptJoinResponse::Failed))
                .wrapped_msg(ErrorKind::CommunicationChannel, "Error sending quorum entrance response to reconfiguration protocol")?;
        } else {
            self.reconf_tx.send(QuorumReconfigurationResponse::QuorumAttemptJoinResponse(QuorumAttemptJoinResponse::Success))
                .wrapped_msg(ErrorKind::CommunicationChannel, "Error sending quorum entrance response to reconfiguration protocol")?;
        }

        Ok(())
    }

    /// Send the result of attempting to join the quorum to the reconfiguration protocol, so that
    /// it can proceed with execution
    fn reply_to_quorum_entrance_request(&mut self, node_id: NodeId, failed_reason: Option<AlterationFailReason>) -> Result<()> {
        info!("{:?} // Sending quorum entrance response to reconfiguration protocol with success: {:?}", self.id(), failed_reason);

        match failed_reason {
            None => {
                self.reconf_tx.send(QuorumReconfigurationResponse::QuorumAlterationResponse(QuorumAlterationResponse::Successful(node_id)))
                    .wrapped_msg(ErrorKind::CommunicationChannel, "Error sending quorum entrance response to reconfiguration protocol")?;
            }
            Some(fail_reason) => {
                self.reconf_tx.send(QuorumReconfigurationResponse::QuorumAlterationResponse(QuorumAlterationResponse::Failed(node_id, fail_reason)))
                    .wrapped_msg(ErrorKind::CommunicationChannel, "Error sending quorum entrance response to reconfiguration protocol")?;
            }
        }

        Ok(())
    }
}

impl QuorumReconfig {
    /// Attempt to register the node
    fn append_pending_node_join(&mut self, node: NodeId) -> bool {
        if let None = self.node_pending_join {
            self.node_pending_join = Some(node);
            true
        } else {
            false
        }
    }

    fn pop_pending_node_join(&mut self) -> Option<NodeId> {
        std::mem::replace(&mut self.node_pending_join, None)
    }
}

/// Checkpoint period.
///
/// Every `PERIOD` messages, the message log is cleared,
/// and a new log checkpoint is initiated.
/// TODO: Move this to an env variable as it can be highly dependent on the service implemented on top of it
pub const CHECKPOINT_PERIOD: u32 = 1000;

impl<R> PartialEq for ReplicaPhase<R> where {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ReplicaPhase::OrderingProtocol, ReplicaPhase::OrderingProtocol) => true,
            (ReplicaPhase::StateTransferProtocol { .. }, ReplicaPhase::StateTransferProtocol { .. }) => true,
            (_, _) => false
        }
    }
}

impl<R> Debug for ReplicaPhase<R> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplicaPhase::OrderingProtocol => {
                write!(f, "OrderingProtocol")
            }
            ReplicaPhase::StateTransferProtocol { log_transfer, state_transfer } => {
                write!(f, "StateTransferProtocol {:?}", state_transfer)?;

                write!(f, ", Log transfer protocol")?;

                if let Some((seq, seq_2, _)) = log_transfer {
                    write!(f, " from {:?} to {:?}", seq, seq_2)
                } else {
                    write!(f, " None")
                }
            }
        }
    }
}