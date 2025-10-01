use anyhow::anyhow;
use lazy_static::lazy_static;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};

use atlas_common::error::*;

use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::{Header, StoredMessage};
use atlas_core::ordering_protocol::loggable::{LoggableOrderProtocol, PProof};
use atlas_core::ordering_protocol::networking::serialize::NetworkView;
use atlas_core::ordering_protocol::OrderingProtocol;

use atlas_core::timeouts::timeout::{ModTimeout, TimeoutModHandle, TimeoutableMod};
use atlas_core::timeouts::TimeoutID;
use atlas_logging_core::decision_log::serialize::OrderProtocolLog;
use atlas_logging_core::decision_log::{DecLog, DecisionLog};
use atlas_logging_core::log_transfer::networking::LogTransferSendNode;
use atlas_logging_core::log_transfer::{
    LTPollResult, LTResult, LTTimeoutResult, LogTM, LogTransferProtocol,
    LogTransferProtocolInitializer,
};
use atlas_logging_core::persistent_log::PersistentDecisionLog;
use atlas_metrics::metrics::metric_duration;

use crate::config::LogTransferConfig;
use crate::messages::serialize::LTMsg;
use crate::messages::{LTMessage, LogTransferMessageKind};
use crate::metrics::{LOG_TRANSFER_LOG_CLONE_TIME_ID, LOG_TRANSFER_PROOFS_CLONE_TIME_ID};

pub mod config;
pub mod messages;
pub mod metrics;

#[derive(Clone)]
struct FetchSeqNoData<P> {
    received_initial_seq: BTreeMap<SeqNo, usize>,
    last_seq: SeqNo,
    last_seq_proof: Option<P>,
}

#[derive(Clone)]
struct FetchSeqNo {
    first_seq: SeqNo,
    last_seq: SeqNo,
}

///FIXME: this should copy as it will copy all know proofs everytime
/// This is fine atm since we aren't using this but we should fix it
#[derive(Clone)]
struct FetchingLogData<P> {
    first_seq: SeqNo,
    last_seq: SeqNo,
    log: Vec<Option<P>>,
}

enum LogTransferState<P, D> {
    Init,
    /// We are currently fetching the latest sequence number
    FetchingSeqNo(usize, FetchSeqNoData<P>),
    /// We are currently fetching the log
    FetchingLogParts(usize, FetchingLogData<P>),
    FetchingLog(usize, FetchSeqNo, Option<D>),
}

pub type Serialization<LT, D, OP, POP> = <LT as LogTransferProtocol<D, OP, POP>>::Serialization;

pub struct CollabLogTransfer<D, OP, DL, NT, PL, EX>
where
    D: SerMsg + 'static,
    OP: LoggableOrderProtocol<D>,
    DL: DecisionLog<D, OP>,
{
    // The current sequence number of the log transfer protocol
    curr_seq: SeqNo,
    // The default timeout for the log transfer protocol
    default_timeout: Duration,
    /// The current state of the log transfer protocol
    log_transfer_state: LogTransferState<
        PProof<D, OP::Serialization, OP::PersistableTypes>,
        DecLog<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
    >,
    /// Reference to the timeouts module
    timeouts: TimeoutModHandle,
    /// Reference to the persistent log
    persistent_log: PL,
    node: Arc<NT>,
    _p: PhantomData<fn() -> EX>,
}

impl<D, OP, DL, NT, PL, EX> CollabLogTransfer<D, OP, DL, NT, PL, EX>
where
    D: SerMsg + 'static,
    OP: LoggableOrderProtocol<D>,
    DL: DecisionLog<D, OP>,
{
    fn curr_seq(&self) -> SeqNo {
        self.curr_seq
    }

    fn next_seq(&mut self) -> SeqNo {
        self.curr_seq += SeqNo::ONE;

        self.curr_seq
    }

    fn request_entire_log<V>(
        &mut self,
        _decision_log: &DL,
        view: V,
        _fetch_data: FetchSeqNoData<PProof<D, OP::Serialization, OP::PersistableTypes>>,
    ) -> Result<()>
    where
        V: NetworkView,
        NT: LogTransferSendNode<
            D,
            OP::Serialization,
            LTMsg<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        >,
    {
        let next_seq = self.next_seq();
        let message = LTMessage::new(next_seq, LogTransferMessageKind::RequestLog);

        let _ = self
            .node
            .broadcast_signed(message, view.quorum_members().clone().into_iter());

        Ok(())
    }

    fn process_log_state_req(
        &self,
        decision_log: &mut DL,
        header: Header,
        message: LTMessage<
            PProof<D, OP::Serialization, OP::PersistableTypes>,
            DecLog<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        >,
    ) -> Result<()>
    where
        PL: PersistentDecisionLog<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        NT: LogTransferSendNode<
            D,
            OP::Serialization,
            LTMsg<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        >,
    {
        let log = decision_log.current_log()?;

        let first_seq = log.first_seq();
        let last_seq = decision_log.sequence_number_with_proof()?;

        let response_msg = if let Some(first_seq) = first_seq {
            LogTransferMessageKind::ReplyLogState(Some((first_seq, last_seq.unwrap())))
        } else {
            LogTransferMessageKind::ReplyLogState(None)
        };

        let message = LTMessage::new(message.sequence_number(), response_msg);

        debug!(
            "{:?} // Sending log state {:?} to {:?}",
            self.node.id(),
            message,
            header.from()
        );

        let _ = self.node.send_signed(message, header.from(), true);

        Ok(())
    }

    fn process_log_parts_request(
        &self,
        decision_log: &mut DL,
        header: Header,
        message: LTMessage<
            PProof<D, OP::Serialization, OP::PersistableTypes>,
            DecLog<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        >,
    ) -> Result<()>
    where
        PL: PersistentDecisionLog<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        NT: LogTransferSendNode<
            D,
            OP::Serialization,
            LTMsg<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        >,
    {
        match message.kind() {
            LogTransferMessageKind::RequestProofs(log_parts) => {
                let mut parts = Vec::with_capacity(log_parts.len());

                let start = Instant::now();

                for part_seq in log_parts {
                    if let Some(part) = decision_log.get_proof(*part_seq)? {
                        parts.push((*part_seq, part))
                    } else {
                        error!(
                            "Request for log part {:?} failed as we do not possess it",
                            *part_seq
                        );
                    }
                }

                metric_duration(LOG_TRANSFER_PROOFS_CLONE_TIME_ID, start.elapsed());

                let message_kind = LogTransferMessageKind::ReplyLogParts(parts);

                let response_msg = LTMessage::new(message.sequence_number(), message_kind);

                let _ = self.node.send_signed(response_msg, header.from(), true);
            }
            _ => {
                unreachable!()
            }
        }

        Ok(())
    }

    fn process_log_request(
        &self,
        decision_log: &mut DL,
        header: Header,
        message: LTMessage<
            PProof<D, OP::Serialization, OP::PersistableTypes>,
            DecLog<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        >,
    ) -> Result<()>
    where
        PL: PersistentDecisionLog<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        NT: LogTransferSendNode<
            D,
            OP::Serialization,
            LTMsg<D, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
        >,
    {
        let start = Instant::now();

        let decision_log = decision_log.snapshot_log()?;

        metric_duration(LOG_TRANSFER_LOG_CLONE_TIME_ID, start.elapsed());

        let message_kind = LogTransferMessageKind::ReplyLog(decision_log);

        let message = LTMessage::new(message.sequence_number(), message_kind);

        let _ = self.node.send_signed(message, header.from(), true);

        Ok(())
    }

    fn timed_out(&mut self, seq: SeqNo) -> LTTimeoutResult {
        if seq != self.curr_seq {
            return LTTimeoutResult::NotNeeded;
        }

        match self.log_transfer_state {
            LogTransferState::Init => LTTimeoutResult::NotNeeded,
            LogTransferState::FetchingSeqNo(_, _)
            | LogTransferState::FetchingLogParts(_, _)
            | LogTransferState::FetchingLog(_, _, _) => LTTimeoutResult::RunLTP,
        }
    }
}

impl<RQ, OP, DL, NT, PL, EX> LogTransferProtocolInitializer<RQ, OP, DL, PL, EX, NT>
    for CollabLogTransfer<RQ, OP, DL, NT, PL, EX>
where
    RQ: SerMsg + 'static,
    OP: LoggableOrderProtocol<RQ>,
    DL: DecisionLog<RQ, OP>,
    PL: PersistentDecisionLog<RQ, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
    NT: LogTransferSendNode<
        RQ,
        OP::Serialization,
        LTMsg<RQ, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
    >,
{
    fn initialize(
        config: Self::Config,
        timeout: TimeoutModHandle,
        node: Arc<NT>,
        log: PL,
    ) -> Result<Self>
    where
        Self: Sized,
        PL: PersistentDecisionLog<
            RQ,
            OP::Serialization,
            OP::PersistableTypes,
            DL::LogSerialization,
        >,
        NT: LogTransferSendNode<RQ, OP::Serialization, Self::Serialization>,
    {
        let LogTransferConfig { timeout_duration } = config;

        let log_transfer = Self {
            curr_seq: SeqNo::ZERO,
            default_timeout: timeout_duration,
            log_transfer_state: LogTransferState::Init,
            timeouts: timeout,
            persistent_log: log,
            node,
            _p: Default::default(),
        };

        Ok(log_transfer)
    }
}

lazy_static! {
    static ref MOD_NAME: Arc<str> = Arc::from("DEFAULT_LOG_TRANSFER");
}

impl<RQ, OP, DL, NT, PL, EX> TimeoutableMod<LTTimeoutResult>
    for CollabLogTransfer<RQ, OP, DL, NT, PL, EX>
where
    RQ: SerMsg + 'static,
    OP: LoggableOrderProtocol<RQ>,
    DL: DecisionLog<RQ, OP>,
    PL: PersistentDecisionLog<RQ, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
    NT: LogTransferSendNode<
        RQ,
        OP::Serialization,
        LTMsg<RQ, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
    >,
{
    fn mod_name() -> Arc<str> {
        MOD_NAME.clone()
    }

    fn handle_timeout(&mut self, _timeout: Vec<ModTimeout>) -> Result<LTTimeoutResult> {
        //TODO: Handle timeouts
        Ok(LTTimeoutResult::NotNeeded)
    }
}

impl<RQ, OP, DL, NT, PL, EX> LogTransferProtocol<RQ, OP, DL>
    for CollabLogTransfer<RQ, OP, DL, NT, PL, EX>
where
    RQ: SerMsg + 'static,
    OP: LoggableOrderProtocol<RQ>,
    DL: DecisionLog<RQ, OP>,
    PL: PersistentDecisionLog<RQ, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
    NT: LogTransferSendNode<
        RQ,
        OP::Serialization,
        LTMsg<RQ, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>,
    >,
{
    type Serialization = LTMsg<RQ, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>;
    type Config = LogTransferConfig;

    fn request_latest_log<V>(&mut self, _decision_log: &mut DL, view: V) -> Result<()>
    where
        V: NetworkView,
    {
        self.log_transfer_state = LogTransferState::FetchingSeqNo(0, FetchSeqNoData::new());

        let lg_seq = self.next_seq();
        let message = LTMessage::new(lg_seq, LogTransferMessageKind::RequestLogState);

        info!(
            "{:?} // Requesting latest consensus seq no with seq {:?}",
            self.node.id(),
            lg_seq
        );

        let _ = self.timeouts.request_timeout(
            TimeoutID::SeqNoBased(message.sequence_number()),
            None,
            self.default_timeout,
            view.quorum(),
            false,
        );

        let _ = self
            .node
            .broadcast_signed(message, view.quorum_members().clone().into_iter());

        Ok(())
    }

    fn poll(
        &mut self,
    ) -> Result<LTPollResult<LogTM<RQ, OP::Serialization, Self::Serialization>, RQ>> {
        Ok(LTPollResult::ReceiveMsg)
    }

    fn handle_off_ctx_message<V>(
        &mut self,
        decision_log: &mut DL,
        view: V,
        message: StoredMessage<LogTM<RQ, OP::Serialization, Self::Serialization>>,
    ) -> Result<()>
    where
        V: NetworkView,
    {
        let (header, message) = message.into_inner();

        debug!(
            "{:?} // Off context Log Transfer Message {:?} from {:?} with seq {:?}",
            self.node.id(),
            message,
            header.from(),
            message.sequence_number()
        );

        match message.kind() {
            LogTransferMessageKind::RequestLogState => {
                let message = message;

                self.process_log_state_req(decision_log, header, message)?;

                return Ok(());
            }
            LogTransferMessageKind::RequestProofs(_log_parts) => {
                let message = message;

                self.process_log_parts_request(decision_log, header, message)?;

                return Ok(());
            }
            LogTransferMessageKind::RequestLog => {
                let message = message;

                self.process_log_request(decision_log, header, message)?;

                return Ok(());
            }
            _ => {}
        }

        let status =
            self.process_message(decision_log, view, StoredMessage::new(header, message))?;

        match status {
            LTResult::NotNeeded | LTResult::Running => (),
            // should not happen...
            _ => {
                return Err(anyhow!(format!(
                    "Invalid state reached while processing log transfer message! {:?}",
                    status
                )));
            }
        }

        Ok(())
    }

    fn process_message<V>(
        &mut self,
        decision_log: &mut DL,
        view: V,
        message: StoredMessage<LogTM<RQ, OP::Serialization, Self::Serialization>>,
    ) -> Result<LTResult<RQ>>
    where
        V: NetworkView,
    {
        let (header, message) = message.into_inner();

        match message.kind() {
            LogTransferMessageKind::RequestLogState => {
                self.process_log_state_req(decision_log, header, message)?;
                return Ok(LTResult::Running);
            }
            LogTransferMessageKind::RequestProofs(_) => {
                self.process_log_parts_request(decision_log, header, message)?;
                return Ok(LTResult::Running);
            }
            LogTransferMessageKind::RequestLog => {
                self.process_log_request(decision_log, header, message)?;
                return Ok(LTResult::Running);
            }
            _ => (),
        }

        if message.sequence_number() != self.curr_seq {
            warn!(
                "{:?} // Received out of order log transfer message {:?} vs curr {:?}",
                self.node.id(),
                message.sequence_number(),
                self.curr_seq
            );

            return Ok(LTResult::Running);
        }

        let _ = self.timeouts.ack_received(
            TimeoutID::SeqNoBased(message.sequence_number()),
            header.from(),
        );

        let lt_state = std::mem::replace(&mut self.log_transfer_state, LogTransferState::Init);

        match lt_state {
            LogTransferState::Init => {
                // Nothing is being done and this isn't a request, so ignore it
                debug!(
                    "{:?} // Received log transfer message {:?} in Init state",
                    self.node.id(),
                    message
                );

                self.log_transfer_state = LogTransferState::Init;

                Ok(LTResult::Ignored)
            }
            LogTransferState::FetchingSeqNo(i, mut curr_state) => {
                match message.into_kind() {
                    LogTransferMessageKind::ReplyLogState(data) => {
                        if let Some((first_seq, (last_seq, last_seq_proof))) = data {
                            if decision_log.verify_sequence_number(last_seq, &last_seq_proof)? {
                                info!(
                                    "{:?} // Received vote for sequence number range {:?} - {:?}",
                                    self.node.id(),
                                    first_seq,
                                    last_seq
                                );

                                let current_count = curr_state
                                    .received_initial_seq
                                    .entry(first_seq)
                                    .or_insert_with(|| 0);

                                *current_count += 1;

                                match last_seq.cmp(&curr_state.last_seq) {
                                    Ordering::Greater => {
                                        curr_state.last_seq = last_seq;
                                        curr_state.last_seq_proof = Some(last_seq_proof);
                                    }
                                    Ordering::Equal | Ordering::Less => {}
                                }
                            } else {
                                //TODO: Handle forgeries?
                                error!("{:?} // Node {:?} has attempt to forge a proof for a log space {:?}", self.node.id(), header.from(), (first_seq, last_seq))
                            }
                        } else {
                            //TODO: Vote for seq no zero?
                        }
                    }
                    _ => {
                        // Drop messages that are not relevant to us at this time

                        self.log_transfer_state = LogTransferState::FetchingSeqNo(i, curr_state);

                        return Ok(LTResult::Running);
                    }
                }

                let i = i + 1;

                if i == view.quorum() {
                    if curr_state.last_seq > decision_log.sequence_number() {
                        let seq = curr_state.last_seq;

                        info!(
                            "{:?} // Installing sequence number and requesting decision log {:?}",
                            self.node.id(),
                            seq
                        );

                        let data = FetchSeqNo::from(&curr_state);

                        self.request_entire_log(decision_log, view, curr_state)?;

                        self.log_transfer_state = LogTransferState::FetchingLog(0, data, None);

                        // this step will allow us to ignore any messages
                        // for older consensus instances we may have had stored;
                        //
                        // after we receive the latest recovery state, we
                        // need to install the then latest sequence no;
                        // this is done with the function
                        // `install_recovery_state` from cst
                        Ok(LTResult::InstallSeq(seq))
                    } else {
                        self.log_transfer_state = LogTransferState::FetchingSeqNo(i, curr_state);

                        info!(
                            "{:?} // No need to request log state, we are up to date",
                            self.node.id()
                        );
                        Ok(LTResult::NotNeeded)
                    }
                } else {
                    self.log_transfer_state = LogTransferState::FetchingSeqNo(i, curr_state);

                    Ok(LTResult::Running)
                }
            }
            LogTransferState::FetchingLog(i, data, current_log) => {
                match message.into_kind() {
                    LogTransferMessageKind::ReplyLog(log) => {
                        //FIXME: Unwraping this first seq is not really the correct thing to do
                        // as the log of the other replica might be empty because he has just checkpointed.
                        // However, the ordering protocol is already at a SeqNo != 0, so we can't just say it's 0.
                        // On the other hand, this will probably never happen as the checkpoint would have to be available (digested) immediately
                        // (before the time it takes to do one consensus decisison) for the replica to get to that position.
                        let first_log_seq = log.first_seq().unwrap_or(SeqNo::ZERO);

                        let last_log_seq = log.sequence_number();

                        if data.first_seq <= first_log_seq {
                            if last_log_seq >= data.last_seq {
                                info!("{:?} // Received log with sequence number {:?} and first sequence number {:?} from {:?}. Accepting log.",
                                        self.node.id(), log.sequence_number(), log.first_seq(), header.from());

                                let requests_to_execute = decision_log.install_log(log)?;

                                self.log_transfer_state = LogTransferState::Init;

                                return Ok(LTResult::LTPFinished(
                                    first_log_seq,
                                    last_log_seq,
                                    requests_to_execute,
                                ));
                            } else {
                                error!("{:?} // Received log with sequence number {:?} but expected {:?} or higher", self.node.id(), log.sequence_number(), last_log_seq);
                            }
                        } else {
                            error!("{:?} // Received log with first sequence number {:?} but expected {:?} or lower", self.node.id(), log.first_seq(), first_log_seq);
                        }
                    }
                    _ => {
                        self.log_transfer_state =
                            LogTransferState::FetchingLog(i, data, current_log);

                        // Drop messages that are not relevant to us at this time
                        return Ok(LTResult::Running);
                    }
                }

                let i = i + 1;

                if i == view.quorum() {
                    self.log_transfer_state = LogTransferState::FetchingLog(i, data, current_log);

                    // If we get quorum messages and still haven't received a correct log, we need to request it again
                    Ok(LTResult::RunLTP)
                } else {
                    self.log_transfer_state = LogTransferState::FetchingLog(i, data, current_log);

                    Ok(LTResult::Running)
                }
            }
            LogTransferState::FetchingLogParts(_, _) => todo!(),
        }
    }
}

//Constructor and getters for FetchSeqNoData
impl<P> FetchSeqNoData<P> {
    fn new() -> Self {
        Self {
            received_initial_seq: BTreeMap::new(),
            last_seq: SeqNo::ZERO,
            last_seq_proof: None,
        }
    }

    fn get_last_seq(&self) -> SeqNo {
        self.last_seq
    }

    fn get_last_seq_proof(&self) -> &Option<P> {
        &self.last_seq_proof
    }

    fn get_received_initial_seq(&self) -> &BTreeMap<SeqNo, usize> {
        &self.received_initial_seq
    }
}

impl<P> From<&FetchSeqNoData<P>> for FetchSeqNo {
    fn from(value: &FetchSeqNoData<P>) -> Self {
        let mut received_votes: Vec<_> = value
            .received_initial_seq
            .iter()
            .map(|(seq, count)| (*seq, *count))
            .collect();

        received_votes.sort_by(|a, b| a.1.cmp(&b.1).reverse());

        let (first_seq, _votes) = received_votes.swap_remove(0);

        //TODO: Check if votes > f+1 ?

        Self {
            first_seq,
            last_seq: value.last_seq,
        }
    }
}
