use std::collections::VecDeque;

use thiserror::Error;
use tracing::{debug, error, info, warn};

use atlas_common::error::*;
use atlas_common::Err;
use atlas_communication::message::{Header, StoredMessage};
use atlas_core::reconfiguration_protocol::QuorumReconfigurationResponse;

use crate::message::{
    CommittedQC, LockedQC, OperationMessage, QuorumAcceptResponse, QuorumCommitAcceptResponse,
    QuorumCommitResponse, QuorumJoinReconfMessages, QuorumJoinResponse,
};
use crate::quorum_config::network::QuorumConfigNetworkNode;
use crate::quorum_config::operations::quorum_info_op::ObtainQuorumInfoOP;
use crate::quorum_config::operations::{
    Operation, OperationExecutionCandidateError, OperationResponse,
};
use crate::quorum_config::{get_quorum_for_n, InternalNode, QuorumView};

/// The operation to enter the quorum
pub struct EnterQuorumOperation {
    // The threshold of votes we need to receive
    threshold: usize,
    // The messages that we have to analyse once we are ready to
    pending_messages: PendingMessages,
    initial_quorum: QuorumView,

    // The phase of the currently executing operation
    phase: OperationPhase,

    // The locked QC
    locked_qc: Option<LockedQC>,
    // The commit QC
    commit_qc: Option<CommittedQC>,
    // The quorum after we have entered
    new_quorum: Option<QuorumView>,
}

/// The phase of the leader in the quorum reconfiguration protocol
/// In this case, the leader is always the node that is initiating the reconfiguration
///
/// We chose a linear based protocol (instead of a PBFT based one) because we want to
/// prevent the possibility of a replica that is trying to join being able to manipulate this
/// protocol in order to DoS the quorum (which is much easier with PBFT due to the
/// O(n^2) message complexity)
///
/// With this vote and collect based protocol, we assure that the leader is the one with the
/// burden, which makes sense since he is the one that wants to join the protocol
pub enum OperationPhase {
    Waiting,
    // We start by broadcasting our intent to join the quorum
    // Which will yield locked QC responses, where each of the correct
    // Replicas which can accept new quorum nodes will now lock onto the
    // Newest quorum, preventing them from accepting any new requests
    // A lock can be broken in two ways: Either by timing out a given request
    // Or by receiving a lockedQC with 2f+1 members corresponding to the same sequence number
    LockingQC(Vec<StoredMessage<QuorumAcceptResponse>>, usize),
    // After a correct replica receives a correct lockedQC, it will commit that QC
    // And send it's commit confirmation to the leader (us).
    // When we have 2f+1 of these commit confirmations, we can move to the next and
    // Final phase
    // In this phase, we bcast the commit QC (composed of 2f+1 commit certificates).
    // This will make all correct replicas move to the new quorum
    // Any replica in a SeqNo < SeqNo(commitQC) even if it has not locked the quorum previously,
    // Should commit and move to this new quorum as it has received proof that the rest of the
    // Quorum has already done so
    CommittingQC(Vec<StoredMessage<QuorumCommitAcceptResponse>>, usize),

    Done,
}

/// Messages that are pending since we are not ready to receive them
struct PendingMessages {
    // The commit messages we have received
    pending_commit_message: VecDeque<StoredMessage<QuorumJoinReconfMessages>>,
}

pub enum VoteResult {
    Processing,
    RejectionsMet,
}

impl EnterQuorumOperation {
    pub(super) const IS_IN_QUORUM: &'static str = "IS_IN_QUORUM";

    pub(crate) fn initialize(node: &InternalNode) -> Self {
        let current_view = node.observer().current_view();

        Self {
            threshold: get_quorum_for_n(current_view.quorum_members().len()),
            initial_quorum: current_view,
            pending_messages: Default::default(),
            phase: OperationPhase::Waiting,
            locked_qc: None,
            commit_qc: None,
            new_quorum: None,
        }
    }

    fn unwrap_operation_message(message: OperationMessage) -> QuorumJoinReconfMessages {
        match message {
            OperationMessage::QuorumReconfiguration(message) => message,
            _ => unreachable!("Received non quorum join reconf message in enter quorum operation"),
        }
    }

    pub fn handle_locked_vote_received<NT>(
        &mut self,
        _node: &mut InternalNode,
        network: &NT,
        header: Header,
        vote: QuorumJoinResponse,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match &mut self.phase {
            OperationPhase::Waiting => {
                error!("Received locked vote while we are still in waiting phase (have not yet contacted anyone). This should not be possible.");
            }
            OperationPhase::LockingQC(received_votes, rejects) => {
                match vote {
                    QuorumJoinResponse::Accepted(accept_vote) => {
                        //TODO: assure this vote matches the votes we are expecting
                        received_votes.push(StoredMessage::new(header, accept_vote));

                        if received_votes.len() >= self.threshold {
                            info!("Received enough locked votes, moving to commit phase and broadcasting the locked QC");

                            let phase = std::mem::replace(
                                &mut self.phase,
                                OperationPhase::CommittingQC(Default::default(), 0),
                            );

                            let locked_qc = LockedQC::new(phase.unwrap_locked_qc());

                            self.locked_qc = Some(locked_qc.clone());

                            let op_message_type = OperationMessage::QuorumReconfiguration(
                                QuorumJoinReconfMessages::CommitQuorum(locked_qc),
                            );

                            let _ = network.broadcast_quorum_message(
                                op_message_type,
                                self.initial_quorum.quorum_members().clone().into_iter(),
                            );
                        } else {
                            info!("Received locked vote, but not enough to move to commit phase. Waiting for more votes.");
                        }
                    }
                    QuorumJoinResponse::Rejected(rejection_reason) => {
                        warn!(
                            "Received rejection vote from {:?} with reason: {:?}",
                            header.from(),
                            rejection_reason
                        );

                        *rejects += 1;
                    }
                }
            }
            OperationPhase::CommittingQC(_, _) => {
                debug!("Received locked vote while we are in commit phase. Ignoring message. From: {:?}", header.from());
            }
            OperationPhase::Done => {
                debug!("Received locked vote while we are done with the protocol. Ignoring message. From: {:?}", header.from());
            }
        }

        Ok(OperationResponse::Processing)
    }

    pub fn handle_commit_vote_received<NT>(
        &mut self,
        _node: &mut InternalNode,
        network: &NT,
        header: Header,
        vote: QuorumCommitResponse,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match &mut self.phase {
            OperationPhase::Waiting => {
                error!("Received locked vote while we are still in waiting phase (have not yet contacted anyone). This should not be possible.");
            }
            OperationPhase::LockingQC(_, _) => {
                info!("Received commit vote while we are in locking phase. Pushing message into pending queue. From: {:?}", header.from());

                self.pending_messages
                    .push_pending_message(StoredMessage::new(
                        header,
                        QuorumJoinReconfMessages::CommitQuorumResponse(vote),
                    ));
            }
            OperationPhase::CommittingQC(accepts, rejections) => match vote {
                QuorumCommitResponse::Accepted(accept_vote) => {
                    let locked_qc = self
                        .locked_qc
                        .as_ref()
                        .expect("We should have a locked QC at this point");

                    if *locked_qc != *accept_vote.qc() {
                        error!("Received commit vote with different QC than the one we locked. This should not be possible.");

                        return Err!(JoinExecErr::LockedQCDoesNotMatchOurs);
                    } else if *locked_qc.quorum() != *accept_vote.view() {
                        error!("Received commit vote with different quorum than the one we locked. This should not be possible.");

                        return Err!(JoinExecErr::ViewDoesNotMatchOurs);
                    } else {
                        accepts.push(StoredMessage::new(header, accept_vote));
                    }

                    if accepts.len() >= self.threshold {
                        info!("Received enough commit votes, moving to done phase and broadcasting the commit QC");

                        let phase = std::mem::replace(&mut self.phase, OperationPhase::Done);

                        let commit_qc = CommittedQC::new(phase.unwrap_commit_qc());

                        self.commit_qc = Some(commit_qc.clone());
                        self.new_quorum = Some(commit_qc.quorum().clone());

                        let op_message_type = OperationMessage::QuorumReconfiguration(
                            QuorumJoinReconfMessages::Decided(commit_qc),
                        );

                        let _ = network.broadcast_quorum_message(
                            op_message_type,
                            self.initial_quorum.quorum_members().clone().into_iter(),
                        );

                        return Ok(OperationResponse::Completed);
                    }
                }
                QuorumCommitResponse::Rejected(reject_reason) => {
                    warn!(
                        "Received rejection vote from {:?} with reason: {:?}",
                        header.from(),
                        reject_reason
                    );

                    *rejections += 1;
                }
            },
            OperationPhase::Done => {}
        }

        Ok(OperationResponse::Processing)
    }
}

impl Operation for EnterQuorumOperation {
    const OP_NAME: &'static str = "ENTER_QUORUM";

    fn can_execute(
        observer: &InternalNode,
    ) -> std::result::Result<(), OperationExecutionCandidateError> {
        // In order to join, we must first obtain the quorum information
        // TODO: Maybe also add a minimum time since last execution?

        let last_execution: Option<&u128> = observer
            .data()
            .get(ObtainQuorumInfoOP::OP_NAME, ObtainQuorumInfoOP::LAST_EXEC);

        if last_execution.is_none() {
            return Err(OperationExecutionCandidateError::RequirementsNotMet(
                format!("{}", JoinExecErr::QuorumInformationMustBeObtained),
            ));
        }

        Ok(())
    }

    fn iterate<NT>(&mut self, node: &mut InternalNode, network: &NT) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        if let OperationPhase::Waiting = &self.phase {
            let current_view = node.observer.current_view();

            info!(
                "Broadcasting enter quorum request to known quorum: {:?}",
                current_view
            );

            self.phase = OperationPhase::LockingQC(Default::default(), 0);

            let quorum_join = QuorumJoinReconfMessages::RequestJoinQuorum(current_view.clone());

            let message_type = OperationMessage::QuorumReconfiguration(quorum_join);

            let _ = network.broadcast_quorum_message(
                message_type,
                current_view.quorum_members().clone().into_iter(),
            );
        } else if let OperationPhase::CommittingQC(_, _) = &self.phase {
            if self.pending_messages.has_pending_messages() {
                let message = self.pending_messages.pop_pending_message().unwrap();

                let (header, message) = message.into_inner();

                return self.handle_received_message(
                    node,
                    network,
                    header,
                    OperationMessage::QuorumReconfiguration(message),
                );
            }
        }

        Ok(OperationResponse::Processing)
    }

    fn handle_received_message<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        header: Header,
        message: OperationMessage,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        let message = Self::unwrap_operation_message(message);

        let msg_result = match message {
            QuorumJoinReconfMessages::RequestJoinQuorum(_) => {
                error!("Received request join quorum message while we are the ones requesting information. Ignoring.");

                return Ok(OperationResponse::Processing);
            }
            QuorumJoinReconfMessages::LockedQuorumResponse(vote) => {
                self.handle_locked_vote_received(node, network, header, vote)
            }
            QuorumJoinReconfMessages::CommitQuorum(_) => {
                error!("Received commit quorum message while we are the ones requesting information. Ignoring.");

                return Ok(OperationResponse::Processing);
            }
            QuorumJoinReconfMessages::CommitQuorumResponse(vote) => {
                self.handle_commit_vote_received(node, network, header, vote)
            }
            QuorumJoinReconfMessages::Decided(_) => {
                error!("Received decided message while we are the ones requesting information. Ignoring.");

                return Ok(OperationResponse::Processing);
            }
        };

        msg_result
    }

    fn handle_quorum_response<NT>(
        &mut self,
        _node: &mut InternalNode,
        _network: &NT,
        _quorum_response: QuorumReconfigurationResponse,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        todo!()
    }

    fn finish<NT>(mut self, observer: &mut InternalNode, _network: &NT) -> Result<()>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        observer
            .observer()
            .install_quorum_view(self.new_quorum.take().unwrap());

        Ok(())
    }
}

impl PendingMessages {
    pub fn new() -> Self {
        Self {
            pending_commit_message: Default::default(),
        }
    }

    pub fn has_pending_messages(&self) -> bool {
        !self.pending_commit_message.is_empty()
    }

    pub fn pop_pending_message(&mut self) -> Option<StoredMessage<QuorumJoinReconfMessages>> {
        self.pending_commit_message.pop_front()
    }

    pub fn push_pending_message(&mut self, message: StoredMessage<QuorumJoinReconfMessages>) {
        match message.message() {
            QuorumJoinReconfMessages::CommitQuorumResponse(_) => {
                self.pending_commit_message.push_back(message);
            }
            _ => {
                error!("Received unexpected message while not ready to receive it. (Type does not match what we are expecting)");
            }
        }
    }
}

impl Default for PendingMessages {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationPhase {
    pub fn unwrap_locked_qc(self) -> Vec<StoredMessage<QuorumAcceptResponse>> {
        match self {
            OperationPhase::LockingQC(locked_qc, _) => locked_qc,
            _ => unreachable!("Called unwrap_locked_qc on a phase that is not locking qc"),
        }
    }

    pub fn unwrap_commit_qc(self) -> Vec<StoredMessage<QuorumCommitAcceptResponse>> {
        match self {
            OperationPhase::CommittingQC(commit_qc, _) => commit_qc,
            _ => unreachable!("Called unwrap_commit_qc on a phase that is not committing qc"),
        }
    }
}

#[derive(Error, Debug)]
pub enum JoinExecErr {
    #[error("The Obtain quorum information operation is required for this operation to execute")]
    QuorumInformationMustBeObtained,
    #[error("Too many rejects have been received by this operation, making it unfeasible")]
    ReceivedTooManyRejects,
    #[error("The locked QC received in the vote does not match the locked qc we have broadcast")]
    LockedQCDoesNotMatchOurs,
    #[error("The view in the vote does not match our expected view")]
    ViewDoesNotMatchOurs,
}
