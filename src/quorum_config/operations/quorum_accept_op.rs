use either::Either;
use thiserror::Error;
use tracing::{info, warn};

use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::Err;
use atlas_communication::message::Header;
use atlas_core::reconfiguration_protocol::QuorumReconfigurationResponse;

use crate::message::{
    CommittedQC, LockedQC, OperationMessage, QuorumAcceptResponse, QuorumCommitAcceptResponse,
    QuorumCommitResponse, QuorumJoinReconfMessages, QuorumJoinResponse, QuorumRejectionReason,
};
use crate::quorum_config::network::QuorumConfigNetworkNode;
use crate::quorum_config::operations::{
    Operation, OperationExecutionCandidateError, OperationResponse,
};
use crate::quorum_config::{get_f_for_n, get_quorum_for_n, QuorumCert, QuorumCertPart};
use crate::quorum_config::{InternalNode, NodeStatusType, QuorumView};

/// The operation to accept a new node into the quorum
pub struct QuorumAcceptNodeOperation {
    // The node that is entering the quorum
    entering_node: NodeId,

    received_locked_qc: Option<LockedQC>,

    received_committed_qc: Option<CommittedQC>,

    // The current phase of our protocol
    phase: OperationPhase,
}

/// The phase of a replica in the quorum reconfiguration protocol
pub enum OperationPhase {
    Waiting,
    // The new member has requested to join our quorum and we have now locked onto the new quorum
    // And we are waiting to receive the locked QC in order to commit this
    LockedQC(QuorumView),
    // We are committed to the new quorum
    // And are waiting for the leader to send us the commit QC so we can actually execute the
    CommittedQC(QuorumView),
    // We have finished the accept protocol
    Done(QuorumView),
}

impl QuorumAcceptNodeOperation {
    pub fn initialize(entering_node: NodeId) -> Self {
        Self {
            entering_node,
            received_locked_qc: None,
            received_committed_qc: None,
            phase: OperationPhase::Waiting,
        }
    }

    fn unwrap_operation_message(op: OperationMessage) -> QuorumJoinReconfMessages {
        match op {
            OperationMessage::QuorumReconfiguration(msg) => msg,
            _ => unreachable!(
                "We should only receive quorum reconfiguration messages in this operation (accept)"
            ),
        }
    }
}

impl QuorumAcceptNodeOperation {
    /// Handle a request being received
    pub fn handle_request_join_quorum_received<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        header: Header,
        request: QuorumView,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        let current_view = node.observer().current_view();

        info!(
            "Received a request to join the quorum from {:?} with view {:?}",
            header.from(),
            request
        );

        let response: QuorumJoinResponse;

        let result: atlas_common::error::Result<OperationResponse>;

        if let OperationPhase::Waiting = &self.phase {
            result = match current_view
                .sequence_number()
                .index(request.sequence_number())
            {
                Either::Right(0) => {
                    if current_view != request {
                        warn!("The join request made by {:?} is not valid as the provided quorum view does not match our own. {:?} vs {:?}", 
                        header.from(), current_view, request,);

                        response = QuorumJoinResponse::Rejected(
                            QuorumRejectionReason::ViewDoesNotMatch(current_view),
                        );

                        Ok(OperationResponse::CompletedFailed)
                    } else {
                        info!(
                            "Accepting quorum join request from {:?} with view {:?}",
                            header.from(),
                            request
                        );

                        let next_view = current_view.next_with_added_node(
                            header.from(),
                            get_f_for_n(current_view.quorum_members().len() + 1),
                        );

                        self.phase = OperationPhase::LockedQC(next_view.clone());

                        response = QuorumJoinResponse::Accepted(QuorumAcceptResponse::init(
                            next_view.clone(),
                        ));

                        Ok(OperationResponse::Processing)
                    }
                }
                Either::Right(_) => {
                    warn!("The join request made by {:?} is not valid as it's sequence number is too old. {:?} vs {:?}",
                    header.from(), current_view.sequence_number(), request.sequence_number(),);

                    response =
                        QuorumJoinResponse::Rejected(QuorumRejectionReason::SeqNoTooAdvanced(
                            request.sequence_number(),
                            current_view.sequence_number(),
                        ));

                    Ok(OperationResponse::CompletedFailed)
                }
                Either::Left(_) => {
                    warn!("The join request made by {:?} is not valid as it's sequence number is too old. {:?} vs {:?}",
                    header.from(), current_view.sequence_number(), request.sequence_number(),);

                    response = QuorumJoinResponse::Rejected(QuorumRejectionReason::SeqNoTooOld(
                        request.sequence_number(),
                        current_view.sequence_number(),
                    ));

                    Ok(OperationResponse::CompletedFailed)
                }
            };
        } else {
            warn!("Received a join request from {:?} but we have already accepted node {:?} into the quorum. Ignoring the request.",
                header.from(), self.entering_node);

            response = QuorumJoinResponse::Rejected(QuorumRejectionReason::AlreadyAccepting);

            result = Ok(OperationResponse::Processing);
        }

        network.send_quorum_config_message(
            OperationMessage::QuorumReconfiguration(
                QuorumJoinReconfMessages::LockedQuorumResponse(response),
            ),
            header.from(),
        )?;

        result
    }

    /// Handle a locked quorum certificate being received from the network
    pub fn handle_quorum_locked_qc_message<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        header: Header,
        message: LockedQC,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        if let Err(err) = is_valid_qc(&message, &node.observer().current_view()) {
            warn!(
                "Received invalid quorum certificate from {:?} in accept operation",
                header.from()
            );

            let rejected_qc =
                QuorumCommitResponse::Rejected(QuorumRejectionReason::InvalidPart(err.to_string()));

            network.send_quorum_config_message(
                OperationMessage::QuorumReconfiguration(
                    QuorumJoinReconfMessages::CommitQuorumResponse(rejected_qc),
                ),
                header.from(),
            )?;

            return Err!(QuorumAcceptOpError::QCError(err));
        }

        //TODO: Better check the locked quorum certificate

        match &mut self.phase {
            OperationPhase::LockedQC(view) => {
                let view_cpy = view.clone();

                // Whenever we receive a lockedQC message which is valid,
                // We must embrace it, as 2f+1 replicas have already done
                self.phase = OperationPhase::CommittedQC(view_cpy.clone());
                self.received_locked_qc = Some(message.clone());

                if self.entering_node != header.from() {
                    warn!("We have received a locked QC message from node {:?} while we were in the process of accepting {:?}.\
                    Since we were still in locked phase, we have moved to the new entering node", header.from(), self.entering_node);

                    self.entering_node = header.from();
                }

                let accepted_qc = QuorumCommitResponse::Accepted(QuorumCommitAcceptResponse::init(view_cpy, message));

                let op_message_type = OperationMessage::QuorumReconfiguration(QuorumJoinReconfMessages::CommitQuorumResponse(accepted_qc));

                network.send_quorum_config_message(op_message_type, header.from())?;
            }
            OperationPhase::CommittedQC(view) => {
                match message.sequence_number().index(view.sequence_number()) {
                    Either::Left(_) => {
                        info!("We have received a locked QC while we are already in the committing phase with seq no older than our current one.\
                        {:?} vs {:?}, ignoring the message (from node {:?})", message.sequence_number(), view.sequence_number(), header.from());
                    }
                    Either::Right(0) => {
                        if self.entering_node != header.from() {
                            warn!("We have received a locked QC while we are already in the committing phase with seq no equal to our current one.\
                        {:?} vs {:?}, ignoring the message (from node {:?} vs already received {:?}).", message.sequence_number(), view.sequence_number(),
                                header.from(), self.entering_node);
                        } else {
                            info!("We have received a locked QC while we are already in the committing phase with seq no equal to our current one.\
                        {:?} vs {:?}. Probably a duplicate, ignoring (sent by {:?}).", message.sequence_number(), view.sequence_number(), header.from());
                        }
                    }
                    Either::Right(_) => {
                        info!("We have received a locked QC while we are already in the committing phase with seq no newer than our current one.\
                        {:?} vs {:?}, moving to the new locked QC (from node {:?})", message.sequence_number(), view.sequence_number(), header.from());

                        self.phase = OperationPhase::CommittedQC(message.quorum().clone());
                        self.received_locked_qc = Some(message);

                        //TODO: Do we contribute to the commit QC by sending our vote?
                    }
                }
            }
            OperationPhase::Done(view) => {
                info!("We have received a locked QC message while we are already done with the accept operation.\
                {:?} vs {:?}, ignoring the message (from node {:?})", message.sequence_number(), view.sequence_number(), header.from());
            }
            OperationPhase::Waiting => panic!("We have received a locked QC message while we are still waiting for the join request to be received"),
        }

        Ok(OperationResponse::Processing)
    }

    pub fn handle_commit_qc_received<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        header: Header,
        message: CommittedQC,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        if let Err(err) = is_valid_qc(&message, &node.observer().current_view()) {
            warn!(
                "Received invalid quorum certificate from {:?} in accept operation",
                header.from()
            );

            let rejected_qc =
                QuorumCommitResponse::Rejected(QuorumRejectionReason::InvalidPart(err.to_string()));

            network.send_quorum_config_message(
                OperationMessage::QuorumReconfiguration(
                    QuorumJoinReconfMessages::CommitQuorumResponse(rejected_qc),
                ),
                header.from(),
            )?;

            return Err!(QuorumAcceptOpError::QCError(err));
        }

        match &self.phase {
            OperationPhase::LockedQC(view) | OperationPhase::CommittedQC(view) => {
                self.phase = OperationPhase::Done(view.clone());
                self.received_committed_qc = Some(message);

                if self.entering_node != header.from() {
                    warn!("We have received a commit QC message from node {:?} while we were in the process of accepting {:?}.\
                    Since we were still in locked phase, we have moved to the new entering node", header.from(), self.entering_node);

                    self.entering_node = header.from();
                }

                return Ok(OperationResponse::Completed);
            }
            OperationPhase::Done(_view) => {}
            OperationPhase::Waiting => panic!("We have received a commit QC message while we are still waiting for the join request to be received"),
        }

        Ok(OperationResponse::Processing)
    }
}

impl Operation for QuorumAcceptNodeOperation {
    const OP_NAME: &'static str = "ACCEPT_NODE";

    fn can_execute(observer: &InternalNode) -> Result<(), OperationExecutionCandidateError>
    where
        Self: Sized,
    {
        match observer.node_type() {
            NodeStatusType::ClientNode { .. } => {
                Err(OperationExecutionCandidateError::RequirementsNotMet(
                    QuorumAcceptOpError::ClientsNotPermitted.to_string(),
                ))
            }
            NodeStatusType::QuorumNode { .. } => {
                if observer.is_part_of_quorum() {
                    Ok(())
                } else {
                    Err(OperationExecutionCandidateError::RequirementsNotMet(
                        QuorumAcceptOpError::CannotRunAcceptWhileNotInQuorum.to_string(),
                    ))
                }
            }
        }
    }

    fn iterate<NT>(
        &mut self,
        _node: &mut InternalNode,
        _network: &NT,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        Ok(OperationResponse::Processing)
    }

    fn handle_received_message<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        header: Header,
        message: OperationMessage,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        let message = Self::unwrap_operation_message(message);

        match message {
            QuorumJoinReconfMessages::RequestJoinQuorum(view) => {
                self.handle_request_join_quorum_received(node, network, header, view)
            }
            QuorumJoinReconfMessages::CommitQuorum(qc) => {
                self.handle_quorum_locked_qc_message(node, network, header, qc)
            }
            QuorumJoinReconfMessages::Decided(qc) => {
                self.handle_commit_qc_received(node, network, header, qc)
            }
            QuorumJoinReconfMessages::LockedQuorumResponse(_) => {
                Err!(QuorumAcceptOpError::ReceivedLockedQuorumResponseWhilePartOfQuorum)
            }
            QuorumJoinReconfMessages::CommitQuorumResponse(_) => {
                Err!(QuorumAcceptOpError::ReceivedCommitQuorumResponseWhileNotCommitted)
            }
        }
    }

    fn handle_quorum_response<NT>(
        &mut self,
        _node: &mut InternalNode,
        _network: &NT,
        _quorum_response: QuorumReconfigurationResponse,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        todo!()
    }

    fn finish<NT>(
        self,
        observer: &mut InternalNode,
        _network: &NT,
    ) -> atlas_common::error::Result<()>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        observer
            .observer()
            .install_quorum_view(self.received_committed_qc.unwrap().quorum().clone());

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum QuorumAcceptOpError {
    #[error("Cannot run accept operation while we are not a part of the quorum")]
    CannotRunAcceptWhileNotInQuorum,
    #[error("Clients are not allowed to perform this operation")]
    ClientsNotPermitted,
    #[error("Should not receive joined quorum message in accept operation, as it should already be ready")]
    ReceivedJoinQuorumMessage,
    #[error("Received a locked quorum response while we are part of the quorum, which should not be possible")]
    ReceivedLockedQuorumResponseWhilePartOfQuorum,
    #[error("Received a commit quorum message while we are not part of the quorum")]
    ReceivedCommitQuorumResponseWhileNotCommitted,
    #[error("There was an error while verifying the QC {0:?}")]
    QCError(#[from] HandleQCError),
}

/// Check if the given quorum certificate is valid
fn is_valid_qc<QC>(qc: &QC, current_quorum: &QuorumView) -> Result<(), HandleQCError>
where
    QC: QuorumCert,
{
    let needed_quorum = get_quorum_for_n(current_quorum.quorum_members().len());

    if qc.proofs().len() < needed_quorum {
        // The QC does not have enough votes to be valid
        return Err(HandleQCError::NotEnoughVotes(
            needed_quorum,
            qc.proofs().len(),
        ));
    }

    match qc.sequence_number().index(current_quorum.sequence_number()) {
        Either::Left(_) | Either::Right(0) => {
            return Err(HandleQCError::TooOld(
                qc.sequence_number(),
                current_quorum.sequence_number(),
            ));
        }
        Either::Right(1) => {}
        Either::Right(_) => {
            // When we have received a quorum certificate that is ahead of us
            // Then we must assume we are the ones in the wrong, since the QC
            // Contains 2f + 1 votes.

            warn!("We have received a quorum certificate that is ahead of our current situation. {:?} vs current {:?}",
            qc.sequence_number(), current_quorum.sequence_number());
        }
    }

    let quorum_view = qc.quorum();

    for stored_response in qc.proofs() {
        if *stored_response.view() != *quorum_view {
            return Err(HandleQCError::NotAllMatch);
        }
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum HandleQCError {
    #[error("Not enough votes, needed {0}, received {1}")]
    NotEnoughVotes(usize, usize),
    #[error("Not all votes on the received certificate match the same quorum view")]
    NotAllMatch,
    #[error("The sequence number indicated by the request is too old as our quorum is already ahead {0:?} vs {1:?}")]
    TooOld(SeqNo, SeqNo),
}
