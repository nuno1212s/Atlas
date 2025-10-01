use anyhow::Context;
use either::Either;
use getset::Getters;
use std::collections::VecDeque;
use std::fmt::{Debug, Formatter};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, instrument, warn};

use crate::metric::{
    DECISION_LOG_PROCESSED_ID, DEC_LOG_PROCESS_TIME_ID, DEC_LOG_WORK_MSG_TIME_ID,
    DEC_LOG_WORK_QUEUE_SIZE_ID,
};
use atlas_common::channel;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::StoredMessage;
use atlas_core::executor::DecisionExecutorHandle;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::{LoggableOrderProtocol, PProof};
use atlas_core::ordering_protocol::networking::serialize::{NetworkView, OrderingProtocolMessage};
use atlas_core::ordering_protocol::{
    Decision, DecisionAD, DecisionMetadata, ExecutionResult, ProtocolMessage,
};
use atlas_core::request_pre_processing::RequestPreProcessing;
use atlas_core::timeouts::timeout::{ModTimeout, TimeoutModHandle};
use atlas_logging_core::decision_log::{
    DecisionLog, DecisionLogInitializer, LoggedDecision, LoggedDecisionValue,
};
use atlas_logging_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_logging_core::log_transfer::networking::LogTransferSendNode;
use atlas_logging_core::log_transfer::{
    LTResult, LTTimeoutResult, LogTM, LogTransferProtocol, LogTransferProtocolInitializer,
};
use atlas_logging_core::persistent_log::PersistentDecisionLog;
use atlas_metrics::metrics::{metric_duration, metric_increment, metric_store_count};
use atlas_smr_core::exec::WrappedExecHandle;
use atlas_smr_core::request_pre_processing::RequestPreProcessor;
use atlas_smr_core::SMRRawReq;

use crate::server::state_transfer::{StateTransferThreadHandle, StateTransferWorkMessage};
use crate::server::CHECKPOINT_PERIOD;

const CHANNEL_SIZE: usize = 1024;

/// The handle to the decision log thread
///
/// This is used by the replica to communicate updates to the decision log worker
/// We rely on the fact that these queues are ordered and that both threads run sequentially,
/// So the communication between them should match up as if they were all running on the same thread
#[derive(Clone, Getters)]
pub struct DecisionLogHandle<V, RQ, OPM, POT, LTM>
where
    V: NetworkView,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    work_tx: ChannelSyncTx<DLWorkMessage<V, RQ, OPM, POT, LTM>>,
    #[getset(get = "pub")]
    status_rx: ChannelSyncRx<ReplicaWorkResponses>,
}

pub enum DecisionLogWorkMessage<RQ, OPM, POT>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
{
    ClearSequenceNumber(SeqNo),
    ClearUnfinishedDecisions,
    DecisionInformation(
        MaybeVec<
            Decision<DecisionMetadata<RQ, OPM>, DecisionAD<RQ, OPM>, ProtocolMessage<RQ, OPM>, RQ>,
        >,
    ),
    Proof(PProof<RQ, OPM, POT>),
    CheckpointDone(SeqNo),
}

/// Messages that are destined to the replica so it can piece
/// together the current state of the decision log
pub enum ReplicaWorkResponses {
    InstallSeqNo(SeqNo),
    LogTransferFinalized(SeqNo, SeqNo),
    LogTransferNotNeeded(SeqNo, SeqNo),
}

pub enum LogTransferWorkMessage<RQ, OPM, LTM>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    RequestLogTransfer,
    LogTransferMessage(StoredMessage<LogTM<RQ, OPM, LTM>>),
    ReceivedTimeout(Vec<ModTimeout>),
    TransferDone(SeqNo, SeqNo),
}

pub enum DLWorkMessageType<RQ, OPM, POT, LTM>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    DecisionLog(DecisionLogWorkMessage<RQ, OPM, POT>),
    LogTransfer(LogTransferWorkMessage<RQ, OPM, LTM>),
}

pub struct DLWorkMessage<V, RQ, OPM, POT, LTM>
where
    V: NetworkView,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    view: V,
    message: DLWorkMessageType<RQ, OPM, POT, LTM>,
}

pub enum DecisionLogResponseMessage<RQ>
where
    RQ: SerMsg,
{
    InstallSeqNo(SeqNo),
    TransferProtocolFinished(SeqNo, SeqNo, MaybeVec<LoggedDecision<RQ>>),
}

pub enum ActivePhase {
    LogTransfer,
    DecisionLog,
}

/// The work queue for the decision log
pub struct DecisionLogWorkQueue<RQ, OPM, POT>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
{
    work_queue: VecDeque<DecisionLogWorkMessage<RQ, OPM, POT>>,
}

pub struct DecisionLogManager<V, R, OP, DL, LT, NT, PL>
where
    V: NetworkView,
    R: SerMsg,
    OP: LoggableOrderProtocol<SMRRawReq<R>>,
    DL: DecisionLog<SMRRawReq<R>, OP>,
    LT: LogTransferProtocol<SMRRawReq<R>, OP, DL>,
{
    decision_log: DL,
    log_transfer: LT,
    node: Arc<NT>,
    work_receiver: ChannelSyncRx<
        DLWorkMessage<V, SMRRawReq<R>, OP::Serialization, OP::PersistableTypes, LT::Serialization>,
    >,
    order_protocol_tx: ChannelSyncTx<ReplicaWorkResponses>,
    decision_log_pending_queue:
        DecisionLogWorkQueue<SMRRawReq<R>, OP::Serialization, OP::PersistableTypes>,
    active_phase: ActivePhase,
    rq_pre_processor: RequestPreProcessor<SMRRawReq<R>>,
    state_transfer_handle: StateTransferThreadHandle<V>,
    executor_handle: WrappedExecHandle<R>,
    pending_decisions_to_execute: Option<MaybeVec<LoggedDecision<SMRRawReq<R>>>>,
    _ph: PhantomData<fn() -> (V, R, OP, NT, PL)>,
}

impl<V, R, OP, DL, LT, NT, PL> DecisionLogManager<V, R, OP, DL, LT, NT, PL>
where
    V: NetworkView + 'static,
    R: SerMsg,
    OP: LoggableOrderProtocol<SMRRawReq<R>>,
    DL: DecisionLog<SMRRawReq<R>, OP> + Send,
    LT: LogTransferProtocol<SMRRawReq<R>, OP, DL> + Send,
    PL: PersistentDecisionLog<
            SMRRawReq<R>,
            OP::Serialization,
            OP::PersistableTypes,
            DL::LogSerialization,
        > + 'static,
    NT: LogTransferSendNode<SMRRawReq<R>, OP::Serialization, LT::Serialization>,
{
    /// Initialize the decision log
    pub fn initialize_decision_log_mngt(
        configs: (DL::Config, LT::Config),
        persistent_log: PL,
        timeouts: TimeoutModHandle,
        node: Arc<NT>,
        rq_pre_processor: RequestPreProcessor<SMRRawReq<R>>,
        state_transfer_thread_handle: StateTransferThreadHandle<V>,
        execution_handle: WrappedExecHandle<R>,
    ) -> Result<
        DecisionLogHandle<
            V,
            SMRRawReq<R>,
            OP::Serialization,
            OP::PersistableTypes,
            LT::Serialization,
        >,
    >
    where
        NT: LogTransferSendNode<SMRRawReq<R>, OP::Serialization, LT::Serialization> + 'static,
        DL: DecisionLogInitializer<SMRRawReq<R>, OP, PL, WrappedExecHandle<R>>,
        LT: LogTransferProtocolInitializer<SMRRawReq<R>, OP, DL, PL, WrappedExecHandle<R>, NT>,
    {
        let (dl_config, lt_config) = configs;

        let node_id = node.id();

        let (dl_work_tx, dl_work_rx) =
            channel::sync::new_bounded_sync(CHANNEL_SIZE, Some("Decision Log Work Channel"));

        let (rp_work_tx, rp_work_rx) = channel::sync::new_bounded_sync(
            CHANNEL_SIZE,
            Some("Decision Log Replica Resp Channel"),
        );

        let handle = DecisionLogHandle {
            work_tx: dl_work_tx,
            status_rx: rp_work_rx,
        };

        std::thread::Builder::new()
            .name("Decision Log Thread".to_string())
            .spawn(move || {
                let decision = DL::initialize_decision_log(
                    dl_config,
                    persistent_log.clone(),
                    execution_handle.clone(),
                )
                .expect("Failed initialize decision log");

                let log_transfer =
                    LT::initialize(lt_config, timeouts, node.clone(), persistent_log)
                        .expect("Failed to initialize log transfer");

                let mut decision_log_manager = Self {
                    decision_log: decision,
                    log_transfer,
                    node,
                    work_receiver: dl_work_rx,
                    order_protocol_tx: rp_work_tx,
                    decision_log_pending_queue: DecisionLogWorkQueue {
                        work_queue: VecDeque::with_capacity(CHANNEL_SIZE),
                    },
                    active_phase: ActivePhase::LogTransfer,
                    rq_pre_processor,
                    state_transfer_handle: state_transfer_thread_handle,
                    executor_handle: execution_handle,
                    pending_decisions_to_execute: None,
                    _ph: PhantomData::default(),
                };

                loop {
                    if let Err(err) = decision_log_manager.run() {
                        error!(
                            "{:?} Ran into error while running the decision log {:?}",
                            node_id, err
                        );
                    }
                }
            })
            .context("Failed to launch decision log")?;

        Ok(handle)
    }

    fn run(&mut self) -> Result<()> {
        info!("Executing decision log thread loop");

        loop {
            let work = self.work_receiver.recv();

            let start = Instant::now();

            if let Ok(work_message) = work {
                let DLWorkMessage { view, message } = work_message;

                match message {
                    DLWorkMessageType::DecisionLog(dl_message) => {
                        self.handle_decision_log_work(dl_message)
                            .context("Decision log work error")?;
                    }
                    DLWorkMessageType::LogTransfer(lt_message) => {
                        self.handle_log_transfer_work(view, lt_message)
                            .context("Log transfer working error")?;
                    }
                }
            }

            metric_duration(DEC_LOG_PROCESS_TIME_ID, start.elapsed());
        }
    }

    fn poll_pending_decisions(&mut self) -> Result<()> {
        while let Some(head_decision) = self.decision_log_pending_queue.work_queue.pop_front() {
            self.run_decision_log_work_message(head_decision)?
        }

        Ok(())
    }

    #[instrument(skip(self))]
    fn handle_decision_log_work(
        &mut self,
        dl_work: DecisionLogWorkMessage<SMRRawReq<R>, OP::Serialization, OP::PersistableTypes>,
    ) -> Result<()> {
        match self.active_phase {
            ActivePhase::DecisionLog => {
                self.run_decision_log_work_message(dl_work)?;
            }
            ActivePhase::LogTransfer => {
                self.decision_log_pending_queue
                    .work_queue
                    .push_back(dl_work);
            }
        }

        Ok(())
    }

    #[instrument(skip(self))]
    fn handle_log_transfer_work(
        &mut self,
        view: V,
        lt_work: LogTransferWorkMessage<SMRRawReq<R>, OP::Serialization, LT::Serialization>,
    ) -> Result<()> {
        match self.active_phase {
            ActivePhase::LogTransfer => {
                self.run_log_transfer_work_message(view, lt_work)?;
            }
            ActivePhase::DecisionLog => {
                match lt_work {
                    LogTransferWorkMessage::RequestLogTransfer => {
                        self.run_log_transfer_protocol(view)?;
                    }
                    LogTransferWorkMessage::LogTransferMessage(message) => {
                        self.log_transfer.handle_off_ctx_message(
                            &mut self.decision_log,
                            view,
                            message,
                        )?;
                    }
                    LogTransferWorkMessage::ReceivedTimeout(timeouts) => {
                        match self.log_transfer.handle_timeout(timeouts)? {
                            LTTimeoutResult::RunLTP => self.run_log_transfer_protocol(view)?,
                            LTTimeoutResult::NotNeeded => {
                                // Keep executing the decision log...
                            }
                        }
                    }
                    LogTransferWorkMessage::TransferDone(_, _) => {
                        warn!("How can the transfer be done if we are running the decision log")
                    }
                }
            }
        }

        Ok(())
    }

    #[instrument(skip(self))]
    fn run_decision_log_work_message(
        &mut self,
        dl_work: DecisionLogWorkMessage<SMRRawReq<R>, OP::Serialization, OP::PersistableTypes>,
    ) -> Result<()> {
        match dl_work {
            DecisionLogWorkMessage::ClearSequenceNumber(clear_seq_no) => {
                self.decision_log.clear_sequence_number(clear_seq_no)?;
            }
            DecisionLogWorkMessage::ClearUnfinishedDecisions => {
                self.decision_log
                    .clear_decisions_forward(self.decision_log.sequence_number())?;
            }
            DecisionLogWorkMessage::DecisionInformation(decision_info) => {
                for decision in decision_info.into_iter() {
                    let decisions_made =
                        self.decision_log.decision_information_received(decision)?;

                    self.execute_logged_decisions(decisions_made)?;
                }
            }
            DecisionLogWorkMessage::Proof(proof) => {
                self.decision_log.install_proof(proof)?;
            }
            DecisionLogWorkMessage::CheckpointDone(seq) => {
                self.decision_log.state_checkpoint(seq)?;
            }
        }

        metric_increment(DECISION_LOG_PROCESSED_ID, None);

        Ok(())
    }

    #[instrument(skip(self))]
    fn run_log_transfer_work_message(
        &mut self,
        view: V,
        lt_work: LogTransferWorkMessage<SMRRawReq<R>, OP::Serialization, LT::Serialization>,
    ) -> Result<()> {
        match lt_work {
            LogTransferWorkMessage::RequestLogTransfer => {
                self.run_log_transfer_protocol(view)?;
            }
            LogTransferWorkMessage::LogTransferMessage(message) => {
                match self.log_transfer.process_message(
                    &mut self.decision_log,
                    view.clone(),
                    message,
                )? {
                    LTResult::RunLTP => {
                        self.log_transfer
                            .request_latest_log(&mut self.decision_log, view)?;
                    }
                    LTResult::NotNeeded => {
                        info!(
                            "Log transfer protocol is not necessary, running decision log protocol"
                        );

                        let _ = self.order_protocol_tx.send(
                            ReplicaWorkResponses::LogTransferNotNeeded(
                                self.decision_log.first_sequence(),
                                self.decision_log.sequence_number(),
                            ),
                        );
                    }
                    LTResult::Running | LTResult::Ignored => {}
                    LTResult::InstallSeq(seq) => {
                        let _ = self
                            .order_protocol_tx
                            .send(ReplicaWorkResponses::InstallSeqNo(seq.next()));
                    }
                    LTResult::LTPFinished(init_seq, last_se, decisions_to_execute) => {
                        self.pending_decisions_to_execute = Some(decisions_to_execute);

                        let _ = self.order_protocol_tx.send(
                            ReplicaWorkResponses::LogTransferFinalized(init_seq, last_se),
                        );
                    }
                };
            }
            LogTransferWorkMessage::ReceivedTimeout(timeout) => {
                self.log_transfer.handle_timeout(timeout)?;
            }
            LogTransferWorkMessage::TransferDone(start, end) => {
                info!(
                    "Received transfer done order from replica with seq {:?}, ending at {:?}",
                    start, end
                );

                if let Some(decisions) = self.pending_decisions_to_execute.take() {
                    decisions.into_iter().for_each(|decision| {
                        match decision.sequence_number().index(start) {
                            Either::Left(_) => {}
                            Either::Right(_) => {
                                let (_seq, client_rqs, decision) = decision.into_inner();

                                let _ = self.rq_pre_processor.process_decided_batch(client_rqs);

                                match decision {
                                    LoggedDecisionValue::Execute(batch) => {
                                        let _ = self.executor_handle.queue_update(batch);
                                    }
                                    LoggedDecisionValue::ExecutionNotNeeded => {
                                        unreachable!("When installing a log transfer, we require the update batch to deliver")
                                    }
                                }
                            }
                        }
                    });
                }

                self.run_decision_log_protocol()?;
            }
        }

        metric_increment(DECISION_LOG_PROCESSED_ID, None);

        Ok(())
    }

    /// Run the log transfer protocol
    #[instrument(skip(self))]
    fn run_log_transfer_protocol(&mut self, view: V) -> Result<()> {
        self.active_phase = ActivePhase::LogTransfer;

        self.log_transfer
            .request_latest_log(&mut self.decision_log, view)?;

        Ok(())
    }

    #[instrument(skip(self))]
    fn run_decision_log_protocol(&mut self) -> Result<()> {
        self.active_phase = ActivePhase::DecisionLog;

        self.poll_pending_decisions()?;

        Ok(())
    }

    #[instrument(skip(self))]
    fn execute_logged_decisions(
        &mut self,
        decisions: MaybeVec<LoggedDecision<SMRRawReq<R>>>,
    ) -> Result<()> {
        for decision in decisions.into_iter() {
            let (seq, requests, to_batch) = decision.into_inner();

            debug!("Sending decided batch to pre processor: {:?}", seq);

            if let Err(err) = self.rq_pre_processor.process_decided_batch(requests) {
                error!("Error sending decided batch to pre processor: {:?}", err);
            }

            let last_seq_no_u32 = u32::from(seq);

            let checkpoint = if last_seq_no_u32 > 0 && last_seq_no_u32 % CHECKPOINT_PERIOD == 0 {
                //We check that % == 0 so we don't start multiple checkpoints

                let (e_tx, e_rx) = channel::oneshot::new_oneshot_channel();

                debug!(
                    "Checking if checkpoint is needed with state transfer protocol {:?}",
                    seq
                );

                self.state_transfer_handle
                    .send_work_message(StateTransferWorkMessage::ShouldRequestAppState(seq, e_tx));

                debug!("Sent work message to state transfer protocol, awaiting response");

                if let Ok(res) = e_rx.recv() {
                    res
                } else {
                    ExecutionResult::Nil
                }
            } else {
                ExecutionResult::Nil
            };

            debug!("Decided batch to execute: {:?}, queuing update", seq);

            match to_batch {
                LoggedDecisionValue::Execute(requests) => match checkpoint {
                    ExecutionResult::Nil => self.executor_handle.queue_update(requests)?,
                    ExecutionResult::BeginCheckpoint => {
                        self.executor_handle.queue_update_and_get_appstate(
                            WrappedExecHandle::transform_update_batch(requests),
                        )?
                    }
                },
                LoggedDecisionValue::ExecutionNotNeeded => {
                    // When the execution is handled
                }
            }
        }

        Ok(())
    }
}

impl<V, RQ, OPM, POT, LTM> DLWorkMessage<V, RQ, OPM, POT, LTM>
where
    V: NetworkView,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    pub fn initialize_message(view: V, work_msg: DLWorkMessageType<RQ, OPM, POT, LTM>) -> Self {
        Self {
            view,
            message: work_msg,
        }
    }

    pub fn init_log_transfer_message(
        view: V,
        work_msg: LogTransferWorkMessage<RQ, OPM, LTM>,
    ) -> Self {
        Self::initialize_message(view, DLWorkMessageType::LogTransfer(work_msg))
    }

    pub fn init_dec_log_message(view: V, work_msg: DecisionLogWorkMessage<RQ, OPM, POT>) -> Self {
        Self::initialize_message(view, DLWorkMessageType::DecisionLog(work_msg))
    }
}

impl<V, RQ, OPM, POT, LTM> DecisionLogHandle<V, RQ, OPM, POT, LTM>
where
    V: NetworkView,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    pub fn send_work(&self, work_message: DLWorkMessage<V, RQ, OPM, POT, LTM>) {
        let start = Instant::now();

        let _ = self.work_tx.send(work_message);

        metric_duration(DEC_LOG_WORK_MSG_TIME_ID, start.elapsed());
        metric_store_count(DEC_LOG_WORK_QUEUE_SIZE_ID, self.work_tx.len());
    }

    pub fn recv_resp(&self) -> ReplicaWorkResponses {
        self.status_rx.recv().unwrap_or_else(|_| {
            unreachable!("Failed to receive message from the decision log thread")
        })
    }

    pub fn try_to_recv_resp(&self) -> Option<ReplicaWorkResponses> {
        if let Ok(message) = self.status_rx.try_recv() {
            Some(message)
        } else {
            None
        }
    }
}

impl<RQ, OPM, POT> Debug for DecisionLogWorkMessage<RQ, OPM, POT>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POT: PersistentOrderProtocolTypes<RQ, OPM>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionLogWorkMessage::ClearSequenceNumber(seq) => {
                write!(f, "Clear sequence number: {:?}", seq)
            }
            DecisionLogWorkMessage::ClearUnfinishedDecisions => {
                write!(f, "Clear unfinished decisions")
            }
            DecisionLogWorkMessage::DecisionInformation(dec_info) => {
                write!(f, "Decision information: {:?}", dec_info)
            }
            DecisionLogWorkMessage::Proof(proof) => {
                write!(f, "Proof: {:?}", proof.sequence_number())
            }
            DecisionLogWorkMessage::CheckpointDone(seq) => {
                write!(f, "Checkpoint done: {:?}", seq)
            }
        }
    }
}

impl<RQ, OPM, LTM> Debug for LogTransferWorkMessage<RQ, OPM, LTM>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    LTM: LogTransferMessage<RQ, OPM>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LogTransferWorkMessage::RequestLogTransfer => {
                write!(f, "Request log transfer")
            }
            LogTransferWorkMessage::LogTransferMessage(message) => {
                write!(f, "Log transfer message: {:?}", message.header())
            }
            LogTransferWorkMessage::ReceivedTimeout(timeout) => {
                write!(f, "Received timeout: {:?}", timeout)
            }
            LogTransferWorkMessage::TransferDone(start, end) => {
                write!(f, "Transfer done: {:?} - {:?}", start, end)
            }
        }
    }
}
