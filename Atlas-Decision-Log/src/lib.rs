#![allow(incomplete_features)]
#![allow(type_alias_bounds)]
use std::time::Instant;

use either::Either;
use thiserror::Error;
use tracing::{debug, error, info, instrument, trace, Level};

use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_common::Err;
use atlas_core::execution::TExecutorDecisionHandle;
use atlas_core::messages::ClientRqInfo;
use atlas_core::ordering_protocol::decision::{Decision, DecisionPart, DecisionRequestBatch, DecisionRequests};
use atlas_core::ordering_protocol::{DecisionAD, DecisionMetadata, ProtocolMessage};
use atlas_core::ordering_protocol::loggable::{PProof, TLoggableOrderProtocol};
use atlas_core::persistent_log::OperationMode;
use atlas_logging_core::decision_log::serialize::OrderProtocolLog;
use atlas_logging_core::decision_log::{DecLog as LogCoreDecLog, DecisionLogInitializer, DecisionSummaryForPersistence, LoggedDecision, RangeOrderable, TDecisionLog, TDecisionLogPersistenceHelper};
use atlas_logging_core::persistent_log::PersistentDecisionLog;
use atlas_metrics::metrics::metric_duration;

use crate::config::DecLogConfig;
use crate::deciding_log::DecidingLog;
use crate::decision_log::InMemDecisionLog;
use crate::decisions::CompletedDecision;
use crate::metric::DECISION_LOG_CHECKPOINT_TIME_ID;
use crate::serialize::LogSerialization;

pub mod config;
pub mod deciding_log;
pub mod decision_log;
pub mod decisions;
mod metric;
pub mod serialize;

/// Decision log implementation type
/// Named after the Council of Five Hundred in ancient Greece.
/// Boule represents the deliberative body where decisions were recorded and enacted.
pub struct Boule<RQ, OP, PL, EX>
where
    RQ: SerMsg,
    OP: TLoggableOrderProtocol<RQ>,
{
    // The log of decisions that are currently ongoing
    deciding_log: DecidingLog<RQ, OP::Serialization, PL>,
    // The log of decisions that have already been decided since the last checkpoint
    decision_log: InMemDecisionLog<RQ, OP::Serialization, OP::PersistableTypes>,
    // A reference to the persistent log
    persistent_log: PL,
    // An execution handle
    _executor_handle: EX,
}

impl<RQ, OP, PL, EX> Orderable for Boule<RQ, OP, PL, EX>
where
    RQ: SerMsg,
    OP: TLoggableOrderProtocol<RQ>,
{
    fn sequence_number(&self) -> SeqNo {
        self.decision_log.last_execution().unwrap_or(SeqNo::ZERO)
    }
}

impl<RQ, OP, PL, EX> RangeOrderable for Boule<RQ, OP, PL, EX>
where
    RQ: SerMsg,
    OP: TLoggableOrderProtocol<RQ>,
{
    fn first_sequence(&self) -> SeqNo {
        self.decision_log.first_seq().unwrap_or(SeqNo::ZERO)
    }
}

pub type LogSer<RQ: SerMsg, OP: TLoggableOrderProtocol<RQ>> =
    LogSerialization<RQ, OP::Serialization, OP::PersistableTypes>;

pub type Proof<RQ: SerMsg, OP: TLoggableOrderProtocol<RQ>> =
    PProof<RQ, OP::Serialization, OP::PersistableTypes>;

pub type DecLog<RQ: SerMsg, OP: TLoggableOrderProtocol<RQ>> =
    LogCoreDecLog<RQ, OP::Serialization, OP::PersistableTypes, LogSer<RQ, OP>>;

impl<RQ, OP, PL, EX>
    TDecisionLogPersistenceHelper<RQ, OP::Serialization, OP::PersistableTypes, LogSer<RQ, OP>>
    for Boule<RQ, OP, PL, EX>
where
    RQ: SerMsg,
    OP: TLoggableOrderProtocol<RQ>,
    PL: Send,
    EX: Send,
{
    fn init_decision_log(_: (), proofs: Vec<Proof<RQ, OP>>) -> Result<DecLog<RQ, OP>> {
        Ok(InMemDecisionLog::from_ordered_proofs(proofs))
    }

    fn decompose_decision_log(
        dec_log: InMemDecisionLog<RQ, OP::Serialization, OP::PersistableTypes>,
    ) -> ((), Vec<Proof<RQ, OP>>) {
        ((), dec_log.into_proofs())
    }

    fn decompose_decision_log_ref(
        dec_log: &InMemDecisionLog<RQ, OP::Serialization, OP::PersistableTypes>,
    ) -> (&(), Vec<&Proof<RQ, OP>>) {
        let mut proofs = Vec::with_capacity(dec_log.proofs().len());

        for proof in dec_log.proofs() {
            proofs.push(proof);
        }

        (&(), proofs)
    }
}

impl<RQ, OP, PL, EX> DecisionLogInitializer<RQ, OP, PL, EX> for Boule<RQ, OP, PL, EX>
where
    RQ: SerMsg + 'static,
    OP: TLoggableOrderProtocol<RQ>,
    PL: PersistentDecisionLog<
        RQ,
        OP::Serialization,
        OP::PersistableTypes,
        LogSerialization<RQ, OP::Serialization, OP::PersistableTypes>,
    >,
    EX: Send,
{
    #[instrument(skip_all, level = "debug")]
    fn initialize_decision_log(
        config: Self::Config,
        persistent_log: PL,
        executor_handle: EX,
    ) -> Result<Self>
    where
        PL: PersistentDecisionLog<
            RQ,
            OP::Serialization,
            OP::PersistableTypes,
            Self::LogSerialization,
        >,
        EX: TExecutorDecisionHandle<RQ>,
        Self: Sized,
    {
        let dec_log = persistent_log
            .read_decision_log(OperationMode::BlockingSync)?
            .unwrap_or_default();

        let deciding = if let Some(seq) = dec_log.last_execution() {
            DecidingLog::init(
                config.default_ongoing_capacity,
                seq.next(),
                persistent_log.clone(),
            )
        } else {
            DecidingLog::init(
                config.default_ongoing_capacity,
                SeqNo::ZERO,
                persistent_log.clone(),
            )
        };

        Ok(Boule {
            deciding_log: deciding,
            decision_log: dec_log,
            persistent_log,
            _executor_handle: executor_handle,
        })
    }
}

impl<RQ, OP, PL, EX> Boule<RQ, OP, PL, EX>
where
    RQ: SerMsg + 'static,
    OP: TLoggableOrderProtocol<RQ>,
    PL: PersistentDecisionLog<
        RQ,
        OP::Serialization,
        OP::PersistableTypes,
        LogSerialization<RQ, OP::Serialization, OP::PersistableTypes>,
    >,
    EX: Send,
{
    #[instrument(skip_all, level = Level::DEBUG, fields(batch_count = batches.len()))]
    fn execute_decision_from_proofs(
        &mut self,
        batches: MaybeVec<DecisionRequests<RQ>>,
    ) -> Result<MaybeVec<LoggedDecision<RQ>>>
    where
        PL: PersistentDecisionLog<RQ, OP::Serialization, OP::PersistableTypes, LogSer<RQ, OP>>,
    {
        let mut decisions_made = MaybeVec::builder();

        for protocol_decision in batches.into_iter() {
            let (seq, update, client_rqs, _batch_digest) = protocol_decision.into();

            let logging_info = DecisionSummaryForPersistence::Proof(seq);

            let logged_decision =
                self.create_logged_decision(seq, client_rqs, update, logging_info)?;

            decisions_made.push(logged_decision);
        }

        Ok(decisions_made.build())
    }

    fn create_logged_decision(
        &self,
        seq: SeqNo,
        client_rqs: Vec<ClientRqInfo>,
        batch: DecisionRequestBatch<RQ>,
        logged_info: DecisionSummaryForPersistence,
    ) -> Result<LoggedDecision<RQ>>
    where
        PL: PersistentDecisionLog<RQ, OP::Serialization, OP::PersistableTypes, LogSer<RQ, OP>>,
    {
        Ok(
            if let Some(batch) = self
                .persistent_log
                .wait_for_full_persistence(batch, logged_info)?
            {
                LoggedDecision::from_decision_with_execution(seq, client_rqs, batch)
            } else {
                LoggedDecision::from_decision(seq, client_rqs)
            },
        )
    }


    #[instrument(skip_all, level = Level::DEBUG, fields(batch_count = decisions.len()))]
    fn execute_decisions(
        &mut self,
        decisions: Vec<CompletedDecision<RQ, OP::Serialization>>,
    ) -> Result<MaybeVec<LoggedDecision<RQ>>>
    where
        PL: PersistentDecisionLog<RQ, OP::Serialization, OP::PersistableTypes, LogSer<RQ, OP>>,
    {
        debug!(
            "Sending {} decisions to be executed by the execution",
            decisions.len()
        );

        let mut decisions_made = MaybeVec::builder();

        for decision in decisions {
            let (_seq, metadata, additional_data, messages, protocol_decision, logged_info) =
                decision.into();

            let proof = OP::init_proof_from_scm(metadata, additional_data, messages)?;

            self.decision_log.append_proof(proof)?;

            let (seq, batch, client_rqs, _batch_digest) = protocol_decision.into();

            let logged_decision =
                self.create_logged_decision(seq, client_rqs, batch, logged_info)?;

            decisions_made.push(logged_decision);
        }

        Ok(decisions_made.build())
    }
}

impl<RQ, OP, PL, EX> TDecisionLog<RQ, OP> for Boule<RQ, OP, PL, EX>
where
    RQ: SerMsg + 'static,
    OP: TLoggableOrderProtocol<RQ>,
    PL: PersistentDecisionLog<
        RQ,
        OP::Serialization,
        OP::PersistableTypes,
        LogSerialization<RQ, OP::Serialization, OP::PersistableTypes>,
    >,
    EX: Send,
{
    type LogSerialization = LogSerialization<RQ, OP::Serialization, OP::PersistableTypes>;
    type Config = DecLogConfig;

    #[instrument(skip(self), level = Level::DEBUG)]
    fn clear_sequence_number(&mut self, seq: SeqNo) -> Result<()>
where {
        let last_exec = self.decision_log.last_execution().unwrap_or(SeqNo::ZERO);

        match seq.index(last_exec) {
            Either::Left(_) | Either::Right(0) => {
                unreachable!("We are trying to clear a sequence number that has already been decided? How can that be cleared?")
            }
            Either::Right(_) => {
                self.deciding_log.clear_decision_at(seq);
            }
        }

        self.persistent_log
            .write_invalidate(OperationMode::NonBlockingSync(None), seq)?;

        Ok(())
    }

    #[instrument(skip(self), level = Level::DEBUG)]
    fn clear_decisions_forward(&mut self, seq: SeqNo) -> Result<()> {
        self.deciding_log.clear_seq_forward_of(seq);

        Ok(())
    }

    #[instrument(skip(self, proof), level = Level::DEBUG, fields(seq_no = proof.sequence_number().into_u32()))]
    fn install_proof(
        &mut self,
        proof: PProof<RQ, OP::Serialization, OP::PersistableTypes>,
    ) -> Result<MaybeVec<LoggedDecision<RQ>>> {
        if let Some(decision) = self.decision_log.last_decision_ref() {
            match proof.sequence_number().index(decision.sequence_number()) {
                Either::Left(_) | Either::Right(0) => {
                    return Err!(DecisionLogError::AlreadyExistsProof(
                        proof.sequence_number()
                    ));
                }
                Either::Right(1) => {
                    self.decision_log.append_proof(proof.clone())?;
                }
                Either::Right(_) => {
                    return Err!(DecisionLogError::AttemptedDecisionIsAhead(
                        proof.sequence_number()
                    ));
                }
            }
        } else {
            self.decision_log.append_proof(proof.clone())?;
        }

        let protocol_decision = OP::get_requests_in_proof(&proof)?;

        self.persistent_log
            .write_proof(OperationMode::NonBlockingSync(None), proof)?;

        self.deciding_log
            .advance_to_seq(self.decision_log.last_execution().unwrap().next());

        self.execute_decision_from_proofs(MaybeVec::One(protocol_decision))
    }

    #[instrument(skip_all, level = Level::DEBUG, fields(first_seq = dec_log.first_seq().map(SeqNo::into_u32), last_seq = dec_log.sequence_number().into_u32()))]
    fn install_log(&mut self, dec_log: DecLog<RQ, OP>) -> Result<MaybeVec<LoggedDecision<RQ>>> {
        info!(
            "Installing a decision log with bounds {:?} - {:?}. Current bounds are: {:?} - {:?}",
            dec_log.first_seq(),
            dec_log.last_execution(),
            self.decision_log.first_seq(),
            self.decision_log.last_execution()
        );

        self.decision_log = dec_log;

        let mut requests = Vec::new();

        // reset the stored log as we are going to receive
        self.persistent_log
            .reset_log(OperationMode::NonBlockingSync(None))?;

        for proof in self.decision_log.proofs() {
            let protocol_decision = OP::get_requests_in_proof(proof)?;

            requests.push(protocol_decision);

            self.persistent_log
                .write_proof(OperationMode::NonBlockingSync(None), proof.clone())?;
        }

        let last_decision = self.decision_log.last_execution();

        if let Some(seq) = last_decision {
            self.deciding_log.advance_to_seq(seq.next());
        } else {
            // We received an empty decision log? Weird

            self.deciding_log.reset_to_zero();
        }

        self.execute_decision_from_proofs(MaybeVec::from_many(requests))
    }

    #[instrument(skip_all, level = Level::DEBUG, fields(first_seq = self.decision_log.first_seq().map(SeqNo::into_u32), last_seq = self.decision_log.sequence_number().into_u32()))]
    fn snapshot_log(&mut self) -> Result<DecLog<RQ, OP>> {
        Ok(self.decision_log.clone())
    }

    #[instrument(skip_all, level = Level::DEBUG, fields(first_seq = self.decision_log.first_seq().map(SeqNo::into_u32), last_seq = self.decision_log.sequence_number().into_u32()))]
    fn current_log(&self) -> Result<&DecLog<RQ, OP>> {
        Ok(&self.decision_log)
    }

    #[instrument(skip(self), level = Level::DEBUG)]
    fn state_checkpoint(&mut self, seq: SeqNo) -> Result<()> {
        let start = Instant::now();

        let _deleted_client_rqs = self.decision_log.clear_until_seq(seq);

        metric_duration(DECISION_LOG_CHECKPOINT_TIME_ID, start.elapsed());

        Ok(())
    }

    #[instrument(skip(self, proof), level = Level::DEBUG, fields(proof_seq_no = proof.sequence_number().into_u32()))]
    fn verify_sequence_number(
        &self,
        seq_no: SeqNo,
        proof: &PProof<RQ, OP::Serialization, OP::PersistableTypes>,
    ) -> Result<bool> {
        if seq_no != proof.sequence_number() {
            return Ok(false);
        }

        //TODO:

        Ok(true)
    }

    #[instrument(skip_all, level = Level::DEBUG, fields(current_seq = self.decision_log.sequence_number().into_u32()))]
    fn sequence_number_with_proof(&self) -> Result<Option<(SeqNo, Proof<RQ, OP>)>> {
        if let Some(decision) = self.decision_log.last_decision() {
            Ok(Some((decision.sequence_number(), decision)))
        } else {
            Ok(None)
        }
    }

    #[instrument(skip(self), level = Level::DEBUG)]
    fn get_proof(&self, seq: SeqNo) -> Result<Option<Proof<RQ, OP>>> {
        if let Some(decision) = self.decision_log.last_execution() {
            if seq > decision {
                Err!(DecisionLogError::NoProofBySeq(seq, decision))
            } else {
                Ok(self.decision_log.get_proof(seq))
            }
        } else {
            Ok(None)
        }
    }

    #[instrument(skip(self), level = Level::DEBUG)]
    fn decision_information_received(
        &mut self,
        decision_info: Decision<
            DecisionMetadata<RQ, OP::Serialization>,
            DecisionAD<RQ, OP::Serialization>,
            ProtocolMessage<RQ, OP::Serialization>,
            RQ,
        >,
    ) -> Result<MaybeVec<LoggedDecision<RQ>>> {
        let seq = decision_info.sequence_number();

        let index = seq.index(
            self.decision_log
                .last_execution()
                .unwrap_or(SeqNo::ZERO),
        );

        match index {
            Either::Left(_) => {
                error!("Received decision information about a decision that has already been made");
            }
            Either::Right(_index) => {
                trace!("Received information about decision {:?}", decision_info);

                decision_info
                    .into_decision_info()
                    .into_iter()
                    .for_each(|info| match info {
                        DecisionPart::DecisionDone => {
                            self.deciding_log.complete_decision(seq);
                        }
                        DecisionPart::DecisionRequests(req) => {
                            self.deciding_log.handle_requests(seq, req);
                        }
                        DecisionPart::PartialDecisionInformation(messages) => {
                            let (decisions_ad, messages) = messages.into();

                            decisions_ad.into_iter().for_each(|decision_ad| {
                                self.deciding_log
                                    .decision_additional_data(seq, decision_ad);
                            });

                            messages.into_iter().for_each(|message| {
                                self.deciding_log.decision_progressed(seq, message);
                            });
                        }
                        DecisionPart::DecisionMetadata(metadata) => {
                            self.deciding_log.decision_metadata(seq, metadata);
                        }
                    });
            }
        }

        let decisions = self.deciding_log.complete_pending_decisions();

        if decisions.is_empty() {
            Ok(MaybeVec::None)
        } else {
            Ok(self.execute_decisions(decisions)?)
        }
    }
}

#[derive(Error, Debug)]
pub enum DecisionLogError {
    #[error("There is no proof by the seq {0:?}. The last execution was {1:?}")]
    NoProofBySeq(SeqNo, SeqNo),
    #[error("There already exists a decision at the sequence number {0:?}")]
    AlreadyExistsProof(SeqNo),
    #[error("Decision that was attempted to append is ahead of our stored decisions {0:?}")]
    AttemptedDecisionIsAhead(SeqNo),
}
