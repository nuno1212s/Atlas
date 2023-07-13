use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use atlas_core::ordering_protocol::stateful_order_protocol::{StatefulOrderProtocol, DecLog};
use log::{debug, error, info, warn};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::{Header, NetworkMessageKind, StoredMessage};
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_core::messages::{LogTransfer, SystemMessage};
use atlas_core::ordering_protocol::{OrderingProtocol, SerProof, View};
use atlas_core::persistent_log::StatefulOrderingProtocolLog;
use atlas_core::serialize::{NetworkView, OrderingProtocolMessage, OrderProtocolLog, ServiceMsg, StatefulOrderProtocolMessage, StateTransferMessage};
use atlas_core::state_transfer::log_transfer::{LogTM, LogTransferProtocol, LTResult, LTTimeoutResult};
use atlas_core::timeouts::{RqTimeout, TimeoutKind, Timeouts};
use atlas_execution::serialize::ApplicationData;
use atlas_metrics::metrics::metric_duration;
use crate::config::LogTransferConfig;
use crate::messages::{LogTransferMessageKind, LTMessage};
use crate::messages::serialize::LTMsg;
use crate::metrics::{LOG_TRANSFER_LOG_CLONE_TIME_ID, LOG_TRANSFER_PROOFS_CLONE_TIME_ID};

pub mod messages;
pub mod config;
pub mod metrics;

#[derive(Clone)]
struct FetchSeqNoData<V, P> {
    received_initial_seq: BTreeMap<SeqNo, usize>,
    last_seq: SeqNo,
    last_seq_proof: Option<P>,
    view: Option<V>,
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

enum LogTransferState<V, P, D> {
    Init,
    /// We are currently fetching the latest sequence number
    FetchingSeqNo(usize, FetchSeqNoData<V, P>),
    /// We are currently fetching the log
    FetchingLogParts(usize, FetchingLogData<P>),
    FetchingLog(usize, FetchSeqNo, Option<D>),
}

pub type Serialization<LT: LogTransferProtocol<D, OP, NT, PL>, D, OP, NT, PL> = <LT as LogTransferProtocol<D, OP, NT, PL>>::Serialization;

pub struct CollabLogTransfer<D, OP, NT, PL>
    where D: ApplicationData + 'static,
          OP: StatefulOrderProtocol<D, NT, PL> + 'static
{
    // The current sequence number of the log transfer protocol
    curr_seq: SeqNo,

    // The default timeout for the log transfer protocol
    default_timeout: Duration,

    /// The current state of the log transfer protocol
    log_transfer_state: LogTransferState<View<OP::Serialization>, SerProof<OP::Serialization>, DecLog<OP::StateSerialization>>,
    /// Reference to the timeouts module
    timeouts: Timeouts,
    /// Node reference
    node: Arc<NT>,
    /// Reference to the persistent log
    persistent_log: PL,
}

impl<D, OP, NT, PL> CollabLogTransfer<D, OP, NT, PL>
    where D: ApplicationData + 'static,
          OP: StatefulOrderProtocol<D, NT, PL> + 'static {
    fn curr_seq(&self) -> SeqNo {
        self.curr_seq
    }

    fn next_seq(&mut self) -> SeqNo {
        self.curr_seq += SeqNo::ONE;

        self.curr_seq
    }

    fn request_entire_log<ST>(&mut self, order_protocol: &OP, fetch_data: FetchSeqNoData<View<OP::Serialization>, SerProof<OP::Serialization>>) -> Result<()>
        where
            ST: StateTransferMessage + 'static,
            NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Serialization<Self, D, OP, NT, PL>>> {
        let next_seq = self.next_seq();
        let message = LTMessage::new(next_seq, LogTransferMessageKind::RequestLog);

        let view = order_protocol.view();

        self.node.broadcast(SystemMessage::from_log_transfer_message(message), NodeId::targets(0..view.n()));

        Ok(())
    }

    fn process_log_state_req<ST>(&self, order_protocol: &mut OP,
                                 header: Header,
                                 message: LTMessage<View<OP::Serialization>, SerProof<OP::Serialization>, DecLog<OP::StateSerialization>>)
                                 -> Result<()>
        where ST: StateTransferMessage + 'static,
              NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Serialization<Self, D, OP, NT, PL>>>,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization> {
        let log = order_protocol.current_log()?;

        let first_seq = log.first_seq();
        let view = order_protocol.view();
        let last_seq = order_protocol.sequence_number_with_proof()?;

        let response_msg = if let Some(first_seq) = first_seq {
            LogTransferMessageKind::ReplyLogState(view, Some((first_seq, last_seq.unwrap())))
        } else {
            LogTransferMessageKind::ReplyLogState(view, None)
        };

        let message = LTMessage::new(message.sequence_number(), response_msg);

        self.node.send(SystemMessage::from_log_transfer_message(message), header.from(), true);

        Ok(())
    }

    fn process_log_parts_request<ST>(&self, order_protocol: &mut OP,
                                     header: Header,
                                     message: LTMessage<View<OP::Serialization>, SerProof<OP::Serialization>, DecLog<OP::StateSerialization>>)
                                     -> Result<()>
        where ST: StateTransferMessage + 'static,
              NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Serialization<Self, D, OP, NT, PL>>>,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization> {
        match message.kind() {
            LogTransferMessageKind::RequestProofs(log_parts) => {
                let mut parts = Vec::with_capacity(log_parts.len());

                let start = Instant::now();

                for part_seq in log_parts {
                    if let Some(part) = order_protocol.get_proof(*part_seq)? {
                        parts.push((*part_seq, part))
                    } else {
                        error!("Request for log part {:?} failed as we do not possess it", *part_seq);
                    }
                }

                metric_duration(LOG_TRANSFER_PROOFS_CLONE_TIME_ID, start.elapsed());

                let message_kind = LogTransferMessageKind::ReplyLogParts(order_protocol.view(), parts);

                let response_msg = LTMessage::new(message.sequence_number(), message_kind);

                self.node.send(SystemMessage::from_log_transfer_message(response_msg), header.from(), true);
            }
            _ => { unreachable!() }
        }

        Ok(())
    }

    fn process_log_request<ST>(&self, order_protocol: &mut OP,
                               header: Header,
                               message: LTMessage<View<OP::Serialization>, SerProof<OP::Serialization>, DecLog<OP::StateSerialization>>)
                               -> Result<()>
        where
            ST: StateTransferMessage + 'static,
            NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Serialization<Self, D, OP, NT, PL>>>,
            PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization> {
        let start = Instant::now();

        let (view, decision_log) = order_protocol.snapshot_log()?;

        metric_duration(LOG_TRANSFER_LOG_CLONE_TIME_ID, start.elapsed());

        let message_kind = LogTransferMessageKind::ReplyLog(view, decision_log);

        let message = LTMessage::new(message.sequence_number(), message_kind);

        self.node.send(SystemMessage::from_log_transfer_message(message), header.from(), true);

        Ok(())
    }

    fn timed_out(&mut self, seq: SeqNo) -> LTTimeoutResult {
        if seq != self.curr_seq {
            return LTTimeoutResult::NotNeeded;
        }

        return match self.log_transfer_state {
            LogTransferState::Init => {
                LTTimeoutResult::NotNeeded
            }
            LogTransferState::FetchingSeqNo(_, _) | LogTransferState::FetchingLogParts(_, _) | LogTransferState::FetchingLog(_, _, _) => {
                LTTimeoutResult::RunLTP
            }
        };
    }
}

impl<D, OP, NT, PL> LogTransferProtocol<D, OP, NT, PL> for CollabLogTransfer<D, OP, NT, PL>
    where D: ApplicationData + 'static,
          OP: StatefulOrderProtocol<D, NT, PL> + 'static {
    type Serialization = LTMsg<D, OP::Serialization, OP::StateSerialization>;
    type Config = LogTransferConfig;

    fn initialize(config: Self::Config, timeouts: Timeouts, node: Arc<NT>, log: PL) -> Result<Self> where Self: Sized {
        let LogTransferConfig {
            timeout_duration
        } = config;

        let log_transfer = Self {
            curr_seq: SeqNo::ZERO,
            default_timeout: timeout_duration,
            log_transfer_state: LogTransferState::Init,
            timeouts,
            node,
            persistent_log: log,
        };

        Ok(log_transfer)
    }

    fn request_latest_log<ST>(&mut self, order_protocol: &mut OP) -> Result<()>
        where NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Self::Serialization>>,
              ST: StateTransferMessage + 'static,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization> {
        self.log_transfer_state = LogTransferState::FetchingSeqNo(0, FetchSeqNoData::new());

        let lg_seq = self.next_seq();
        let view = order_protocol.view();
        let message = LTMessage::new(lg_seq, LogTransferMessageKind::RequestLogState);

        info!("{:?} // Requesting latest consensus seq no with seq {:?}", self.node.id(), lg_seq);

        self.timeouts.timeout_lt_request(self.default_timeout, view.quorum() as u32, message.sequence_number());

        let targets = NodeId::targets(0..view.n());

        self.node.broadcast(SystemMessage::from_log_transfer_message(message), targets);

        Ok(())
    }

    fn handle_off_ctx_message<ST>(&mut self, order_protocol: &mut OP, message: StoredMessage<LogTransfer<LogTM<Self::Serialization>>>) -> Result<()>
        where NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Self::Serialization>>,
              ST: StateTransferMessage + 'static,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization> {

        let (header, message) = message.into_inner();

        debug!("{:?} // Off context Log Transfer Message {:?} from {:?} with seq {:?}", self.node.id(),message.payload(), header.from(), message.sequence_number());

        match message.payload().kind() {
            LogTransferMessageKind::RequestLogState => {
                let message = message.into_inner();

                self.process_log_state_req(order_protocol, header, message)?;

                return Ok(());
            }
            LogTransferMessageKind::RequestProofs(log_parts) => {
                let message = message.into_inner();

                self.process_log_parts_request(order_protocol, header, message)?;

                return Ok(());
            }
            LogTransferMessageKind::RequestLog => {
                let message = message.into_inner();

                self.process_log_request(order_protocol, header, message)?;

                return Ok(());
            }
            _ => {}
        }

        let status = self.process_message(
            order_protocol,
            StoredMessage::new(header, message),
        )?;

        match status {
            LTResult::NotNeeded | LTResult::Running => (),
            // should not happen...
            _ => {
                return Err(format!("Invalid state reached while processing log transfer message! {:?}", status)).wrapped(ErrorKind::CoreServer);
            }
        }

        Ok(())
    }

    fn process_message<ST>(&mut self, order_protocol: &mut OP, message: StoredMessage<LogTransfer<LogTM<Self::Serialization>>>)
                           -> Result<LTResult<D>>
        where
            ST: StateTransferMessage + 'static,
            NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Self::Serialization>>,
            PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization> {
        let (header, message) = message.into_inner();

        match message.payload().kind() {
            LogTransferMessageKind::RequestLogState => {
                self.process_log_state_req(order_protocol, header, message.into_inner())?;
                return Ok(LTResult::Running);
            }
            LogTransferMessageKind::RequestProofs(_) => {
                self.process_log_parts_request(order_protocol, header, message.into_inner())?;
                return Ok(LTResult::Running);
            }
            LogTransferMessageKind::RequestLog => {
                self.process_log_request(order_protocol, header, message.into_inner())?;
                return Ok(LTResult::Running);
            }
            _ => ()
        }

        if message.sequence_number() != self.curr_seq {
            warn!("{:?} // Received out of order log transfer message {:?} vs curr {:?}", self.node.id(),
                    message.sequence_number(), self.curr_seq);

            return Ok(LTResult::Running);
        }

        self.timeouts.received_log_request(header.from(), message.sequence_number());

        let lt_state = std::mem::replace(&mut self.log_transfer_state, LogTransferState::Init);

        match lt_state {
            LogTransferState::Init => {
                // Nothing is being done and this isn't a request, so ignore it
                debug!("{:?} // Received log transfer message {:?} in Init state", self.node.id(), message.payload());

                self.log_transfer_state = LogTransferState::Init;

                return Ok(LTResult::NotNeeded);
            }
            LogTransferState::FetchingSeqNo(i, mut curr_state) => {
                match message.into_inner().into_kind() {
                    LogTransferMessageKind::ReplyLogState(view, data) => {
                        if let Some((first_seq, (last_seq, last_seq_proof))) = data {
                            if order_protocol.verify_sequence_number(last_seq, &last_seq_proof)? {
                                info!("{:?} // Received vote for sequence number range {:?} - {:?}", self.node.id(), first_seq, last_seq);

                                let current_count = curr_state.received_initial_seq.entry(first_seq).or_insert_with(|| 0);

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

                if i == order_protocol.view().quorum() {
                    if curr_state.last_seq > order_protocol.sequence_number() {
                        let seq = curr_state.last_seq;

                        debug!("{:?} // Installing sequence number and requesting decision log {:?}", self.node.id(), seq);

                        let data = FetchSeqNo::from(&curr_state);

                        // this step will allow us to ignore any messages
                        // for older consensus instances we may have had stored;
                        //
                        // after we receive the latest recovery state, we
                        // need to install the then latest sequence no;
                        // this is done with the function
                        // `install_recovery_state` from cst
                        order_protocol.install_seq_no(seq)?;

                        self.request_entire_log(order_protocol, curr_state)?;

                        self.log_transfer_state = LogTransferState::FetchingLog(0, data, None);

                        return Ok(LTResult::Running);
                    } else {
                        self.log_transfer_state = LogTransferState::FetchingSeqNo(i, curr_state);

                        debug!("{:?} // No need to request log state, we are up to date", self.node.id());
                        return Ok(LTResult::LTPFinished(order_protocol.current_log()?.first_seq().unwrap_or(SeqNo::ZERO), order_protocol.sequence_number(), Vec::new()));
                    }
                } else {
                    self.log_transfer_state = LogTransferState::FetchingSeqNo(i, curr_state);

                    return Ok(LTResult::Running);
                }
            }
            LogTransferState::FetchingLog(i, data, current_log) => {
                match message.into_inner().into_kind() {
                    LogTransferMessageKind::ReplyLog(view, log) => {
                        //FIXME: Unwraping this first seq is not really the correct thing to do
                        // as the log of the other replica might be empty because he has just checkpointed.
                        // However, the ordering protocol is already at a SeqNo != 0, so we can't just say it's 0.
                        // On the other hand, this will probably never happen as the checkpoint would have to be available (digested) immediately
                        // (before the time it takes to do one consensus decisison) for the replica to get to that position.
                        let first_log_seq = log.first_seq().unwrap_or(SeqNo::ZERO);

                        let last_log_seq = log.sequence_number();

                        if data.first_seq <= first_log_seq {
                            if last_log_seq >= data.last_seq {
                                info!("{:?} // Received log with sequence number {:?} and first sequence number {:?} from {:?} in view {:?}. Accepting log.",
                                        self.node.id(), log.sequence_number(), log.first_seq(), header.from(), view);

                                let requests_to_execute = order_protocol.install_state(view, log)?;

                                self.log_transfer_state = LogTransferState::Init;

                                return Ok(LTResult::LTPFinished(first_log_seq, last_log_seq, requests_to_execute));
                            } else {
                                error!("{:?} // Received log with sequence number {:?} but expected {:?} or higher", self.node.id(), log.sequence_number(), last_log_seq);
                            }
                        } else {
                            error!("{:?} // Received log with first sequence number {:?} but expected {:?} or lower", self.node.id(), log.first_seq(), first_log_seq);
                        }
                    }
                    _ => {
                        self.log_transfer_state = LogTransferState::FetchingLog(i, data, current_log);

                        // Drop messages that are not relevant to us at this time
                        return Ok(LTResult::Running);
                    }
                }

                let i = i + 1;

                return if i == order_protocol.view().quorum() {
                    self.log_transfer_state = LogTransferState::FetchingLog(i, data, current_log);

                    // If we get quorum messages and still haven't received a correct log, we need to request it again
                    Ok(LTResult::RunLTP)
                } else {
                    self.log_transfer_state = LogTransferState::FetchingLog(i, data, current_log);

                    Ok(LTResult::Running)
                };
            }
            LogTransferState::FetchingLogParts(_, _) => todo!()
        }
    }

    fn handle_timeout<ST>(&mut self, timeout: Vec<RqTimeout>) -> Result<LTTimeoutResult>
        where ST: StateTransferMessage + 'static,
              NT: ProtocolNetworkNode<ServiceMsg<D, OP::Serialization, ST, Self::Serialization>>,
              PL: StatefulOrderingProtocolLog<OP::Serialization, OP::StateSerialization> {
        for lt_seq in timeout {
            if let TimeoutKind::LogTransfer(lt_seq) = lt_seq.timeout_kind() {
                if let LTTimeoutResult::RunLTP = self.timed_out(*lt_seq) {
                    return Ok(LTTimeoutResult::RunLTP);
                }
            }
        }

        Ok(LTTimeoutResult::NotNeeded)
    }
}

//Constructor and getters for FetchSeqNoData
impl<V, P> FetchSeqNoData<V, P> {
    fn new() -> Self {
        Self {
            received_initial_seq: BTreeMap::new(),
            last_seq: SeqNo::ZERO,
            last_seq_proof: None,
            view: None,
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

impl<V, P> From<&FetchSeqNoData<V, P>> for FetchSeqNo {
    fn from(value: &FetchSeqNoData<V, P>) -> Self {
        let mut received_votes: Vec<_> = value.received_initial_seq.iter().map(|(seq, count)| (*seq, *count)).collect();

        received_votes.sort_by(|a, b| {
            a.1.cmp(&b.1).reverse()
        });

        let (first_seq, votes) = received_votes.swap_remove(0);

        //TODO: Check if votes > f+1 ?

        Self {
            first_seq,
            last_seq: value.last_seq,
        }
    }
}