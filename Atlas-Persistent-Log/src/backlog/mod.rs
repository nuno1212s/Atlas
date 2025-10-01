use std::collections::BTreeMap;

use anyhow::Context;
use log::{error, warn};
use thiserror::Error;

use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::{channel, Err};
use atlas_core::executor::DecisionExecutorHandle;
use atlas_core::ordering_protocol::BatchedDecision;
use atlas_logging_core::decision_log::LoggingDecision;

use crate::ResponseMessage;

///This is made to handle the backlog when the consensus is working faster than the persistent storage layer.
/// It holds update batches that are yet to be executed since they are still waiting for the confirmation of the persistent log
/// This is only needed (and only instantiated) when the persistency mode is strict
pub struct ConsensusBacklog<EX, RQ> {
    rx: ChannelSyncRx<BacklogMessage<RQ>>,

    //Receives messages from the persistent log
    logger_rx: ChannelSyncRx<ResponseMessage>,

    //The handle to the executor
    executor_handle: EX,

    //This is the batch that is currently waiting for it's messages to be persisted
    //Even if we already persisted the consensus instance that came after it (for some reason)
    // We can only deliver it when all the previous ones have been delivered,
    // As it must be ordered
    currently_waiting_for: Option<AwaitingPersistence<RQ>>,

    //Message confirmations that we have already received but pertain to a further ahead consensus instance
    messages_received_ahead: BTreeMap<SeqNo, Vec<ResponseMessage>>,
}

/// Backlogged message information
struct BackloggedMessage<O> {
    decision: BatchedDecision<O>,
    // Information about the decision
    logged_decision: LoggingDecision,
}

/// Decision status on message that is awaiting persistency
pub struct AwaitingPersistence<O> {
    message: BackloggedMessage<O>,
    received_message: LoggedMessages,
}

pub enum LoggedMessages {
    Proof(bool),
    MessagesReceived(Vec<Digest>, Option<SeqNo>),
}

type BacklogMessage<O> = BackloggedMessage<O>;

///A detachable handle so we deliver work to the
/// consensus back log thread
pub struct ConsensusBackLogHandle<O> {
    rq_tx: ChannelSyncTx<BacklogMessage<O>>,
    logger_tx: ChannelSyncTx<ResponseMessage>,
}

impl<O> ConsensusBackLogHandle<O> {
    pub fn logger_tx(&self) -> ChannelSyncTx<ResponseMessage> {
        self.logger_tx.clone()
    }

    /// Queue a decision
    pub fn queue_decision(
        &self,
        batch: BatchedDecision<O>,
        decision: LoggingDecision,
    ) -> Result<()> {
        let message = BackloggedMessage {
            decision: batch,
            logged_decision: decision,
        };

        self.rq_tx
            .send(message)
            .context("Failed to queue decision into backlog")
    }
}

impl<O> Clone for ConsensusBackLogHandle<O> {
    fn clone(&self) -> Self {
        Self {
            rq_tx: self.rq_tx.clone(),
            logger_tx: self.logger_tx.clone(),
        }
    }
}

///This channel size serves as the "buffer" for the amount of consensus instances
///That can be waiting for messages
const CHANNEL_SIZE: usize = 1024;

impl<EX, RQ> ConsensusBacklog<EX, RQ>
where
    EX: DecisionExecutorHandle<RQ>,
    RQ: Send + 'static,
{
    ///Initialize the consensus backlog
    pub fn init_backlog(executor: EX) -> ConsensusBackLogHandle<RQ> {
        let (logger_tx, logger_rx) =
            channel::sync::new_bounded_sync(CHANNEL_SIZE, Some("Backlog Response Message"));

        let (batch_tx, batch_rx) =
            channel::sync::new_bounded_sync(CHANNEL_SIZE, Some("Backlog batch message"));

        let backlog_thread = ConsensusBacklog {
            rx: batch_rx,
            logger_rx,
            executor_handle: executor,
            currently_waiting_for: None,
            messages_received_ahead: BTreeMap::new(),
        };

        backlog_thread.start_thread();

        let handle = ConsensusBackLogHandle {
            rq_tx: batch_tx,
            logger_tx,
        };

        handle
    }

    fn start_thread(self) {
        std::thread::Builder::new()
            .name(format!("Consensus Backlog thread"))
            .spawn(move || {
                self.run();
            })
            .expect("Failed to start consensus backlog thread.");
    }

    fn run(mut self) {
        loop {
            if self.currently_waiting_for.is_some() {
                let notification = match self.logger_rx.recv() {
                    Ok(notification) => notification,
                    Err(_) => break,
                };

                self.handle_received_message(notification);

                if self
                    .currently_waiting_for
                    .as_ref()
                    .unwrap()
                    .is_ready_for_execution()
                {
                    let finished_batch = self.currently_waiting_for.take().unwrap();

                    self.dispatch_batch(finished_batch.into());
                }
            } else {
                let batch_info = match self.rx.recv() {
                    Ok(rcved) => rcved,
                    Err(err) => {
                        error!("{:?}", err);

                        break;
                    }
                };

                let mut awaiting = AwaitingPersistence::from(batch_info);

                self.process_pending_messages_for_current(&mut awaiting);

                if awaiting.is_ready_for_execution() {
                    //If we have already received everything, dispatch the batch immediately
                    self.dispatch_batch(awaiting.into());

                    continue;
                }

                self.currently_waiting_for = Some(awaiting);
            }
        }
    }

    fn handle_received_message(&mut self, notification: ResponseMessage) {
        let info = self.currently_waiting_for.as_mut().unwrap();

        let curr_seq = info.sequence_number();

        match &notification {
            ResponseMessage::WroteMessage(seq, _)
            | ResponseMessage::Proof(seq)
            | ResponseMessage::WroteMetadata(seq) => {
                if curr_seq == *seq {
                    Self::process_incoming_message(info, notification);
                } else {
                    self.process_ahead_message(seq.clone(), notification);
                }
            }
            _ => {}
        }
    }

    fn process_pending_messages_for_current(&mut self, awaiting: &mut AwaitingPersistence<RQ>) {
        let seq_num = awaiting.sequence_number();

        //Remove the messages that we have already received
        let messages_ahead = self.messages_received_ahead.remove(&seq_num);

        if let Some(messages_ahead) = messages_ahead {
            for persisted_message in messages_ahead {
                Self::process_incoming_message(awaiting, persisted_message);
            }
        }
    }

    fn dispatch_batch(&self, batch: BatchedDecision<RQ>) {
        //TODO: Request checkpointing from the executor
        self.executor_handle
            .queue_update(batch)
            .expect("Failed to queue update");
    }

    fn process_ahead_message(&mut self, seq: SeqNo, notification: ResponseMessage) {
        if let Some(received_msg) = self.messages_received_ahead.get_mut(&seq) {
            received_msg.push(notification);
        } else {
            self.messages_received_ahead.insert(seq, vec![notification]);
        }
    }

    fn process_incoming_message(awaiting: &mut AwaitingPersistence<RQ>, msg: ResponseMessage) {
        let result = awaiting.handle_incoming_message(msg);

        match result {
            Ok(result) => {
                if !result {
                    warn!(
                        "Received message for consensus instance {:?} but was not expecting it?",
                        awaiting.sequence_number()
                    );
                }
            }
            Err(err) => {
                error!(
                    "Received message that does not match up with what we were expecting {:?}",
                    err
                );
            }
        }
    }
}

impl<O> Orderable for AwaitingPersistence<O> {
    fn sequence_number(&self) -> SeqNo {
        self.message.decision.sequence_number()
    }
}

impl<O> AwaitingPersistence<O> {
    pub fn is_ready_for_execution(&self) -> bool {
        match &self.received_message {
            LoggedMessages::Proof(opt) => *opt,
            LoggedMessages::MessagesReceived(persistent, metadata) => {
                persistent.is_empty() && metadata.is_none()
            }
        }
    }

    pub fn handle_incoming_message(&mut self, msg: ResponseMessage) -> Result<bool> {
        if self.is_ready_for_execution() {
            return Ok(false);
        }

        let seq_no = self.sequence_number();

        match &mut self.received_message {
            LoggedMessages::Proof(sq_no) => {
                if let ResponseMessage::Proof(seq) = msg {
                    if seq == seq_no && !*sq_no {
                        *sq_no = true;

                        Ok(true)
                    } else {
                        Ok(false)
                    }
                } else {
                    Err!(BacklogError::ReceivedMessageDoesNotMatchExistingBatch)
                }
            }
            LoggedMessages::MessagesReceived(rqs, metadata) => {
                match msg {
                    ResponseMessage::WroteMetadata(_) => {
                        //We don't check the seq no because that is already checked before getting to this point
                        metadata.take();

                        Ok(true)
                    }
                    ResponseMessage::WroteMessage(_, persisted_message) => {
                        match rqs.iter().position(|p| *p == persisted_message) {
                            Some(msg_index) => {
                                rqs.swap_remove(msg_index);

                                Ok(true)
                            }
                            None => Ok(false),
                        }
                    }
                    _ => {
                        Err!(BacklogError::ReceivedMessageDoesNotMatchExistingBatch)
                    }
                }
            }
        }
    }
}

impl<O> Into<BatchedDecision<O>> for AwaitingPersistence<O> {
    fn into(self) -> BatchedDecision<O> {
        self.message.decision
    }
}

impl<O> From<BacklogMessage<O>> for AwaitingPersistence<O> {
    fn from(value: BacklogMessage<O>) -> Self {
        let received = match &value.logged_decision {
            LoggingDecision::Proof(seq) => LoggedMessages::Proof(false),
            LoggingDecision::PartialDecision(seq, digests) => {
                let message_digests = digests
                    .clone()
                    .into_iter()
                    .map(|(node, digest)| digest)
                    .collect();

                LoggedMessages::MessagesReceived(message_digests, Some(*seq))
            }
        };

        AwaitingPersistence {
            message: value,
            received_message: received,
        }
    }
}

#[derive(Error, Debug)]
pub enum BacklogError {
    #[error("Message received does not match up with the batch that we have received.")]
    ReceivedMessageDoesNotMatchExistingBatch,
}
