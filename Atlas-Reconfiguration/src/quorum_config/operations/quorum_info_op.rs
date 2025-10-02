use crate::message::{OperationMessage, QuorumObtainInfoOpMessage, QuorumViewCert};
use crate::quorum_config::network::QuorumConfigNetworkNode;
use crate::quorum_config::operations::{
    Operation, OperationExecutionCandidateError, OperationResponse,
};
use crate::quorum_config::{InternalNode, QuorumView};
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_common::Err;
use atlas_communication::message::{Header, StoredMessage};
use atlas_core::reconfiguration_protocol::QuorumReconfigurationResponse;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use tracing::{debug, error, info};

/// Obtains the quorum information
pub struct ObtainQuorumInfoOP {
    // The current known quorum
    current_known_quorum: Vec<NodeId>,
    // The amount of nodes we have to receive messages from
    // To consider it done
    threshold: usize,
    // The responses that we have received
    received: BTreeSet<NodeId>,
    // The quorum views we have received
    quorum_views: BTreeMap<Digest, Vec<QuorumViewCert>>,
    // The current state of the operation we are currently performing
    state: OperationState,
}

/// The current state of the operation
enum OperationState {
    Waiting,
    ReceivingInfo(usize),
    Done,
}

impl ObtainQuorumInfoOP {
    pub(super) const LAST_EXEC: &'static str = "LAST_EXECUTED";

    pub fn initialize(threshold: usize, current_known_quorum: Vec<NodeId>) -> Self {
        //TODO: Broadcast the quorum information request to the currently known quorum

        Self {
            current_known_quorum,
            threshold,
            received: BTreeSet::new(),
            quorum_views: BTreeMap::new(),
            state: OperationState::Waiting,
        }
    }

    pub fn respond_to_request<NT>(
        node: &InternalNode,
        network: &NT,
        header: Header,
    ) -> atlas_common::error::Result<()>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        let quorum_info = QuorumObtainInfoOpMessage::QuorumInformationResponse(
            node.observer().current_view().clone(),
        );

        let op_message_type = OperationMessage::QuorumInfoOp(quorum_info);

        network.send_quorum_config_message(op_message_type, header.from())
    }

    pub fn finalize(self) -> atlas_common::error::Result<QuorumView> {
        let mut received_messages = Vec::new();

        for (message_digests, messages) in self.quorum_views.iter() {
            received_messages.push((*message_digests, messages.clone()));
        }

        received_messages.sort_by(|(_, a), (_, b)| a.len().cmp(&b.len()).reverse());

        debug!("Processed received messages: {:?}", received_messages);

        if let Some((quorum_digest, quorum_certs)) = received_messages.first() {
            if quorum_certs.len() >= self.threshold {
                let view = quorum_certs.first().unwrap().message().clone();

                Ok(view)
            } else {
                Err!(QuorumObtainInfoError::FailedNotEnoughMatching(
                    self.threshold,
                    quorum_certs.len(),
                    *quorum_digest
                ))
            }
        } else {
            Err!(QuorumObtainInfoError::FailedNoMessagesReceived)
        }
    }

    fn from_operation_message(msg: OperationMessage) -> QuorumObtainInfoOpMessage {
        match msg {
            OperationMessage::QuorumInfoOp(msg) => msg,
            _ => unreachable!("Received wrong message type"),
        }
    }
}

impl Operation for ObtainQuorumInfoOP {
    const OP_NAME: &'static str = "ObtainQuorumInfo";

    fn can_execute(_observer: &InternalNode) -> Result<(), OperationExecutionCandidateError> {
        Ok(())
    }

    fn iterate<NT>(
        &mut self,
        _node: &mut InternalNode,
        network: &NT,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        if let OperationState::Waiting = self.state {
            let op_message = OperationMessage::QuorumInfoOp(
                QuorumObtainInfoOpMessage::RequestInformationMessage,
            );

            let _ = network.broadcast_quorum_message(
                op_message,
                self.current_known_quorum.clone().into_iter(),
            );

            self.state = OperationState::ReceivingInfo(0);
        }

        Ok(OperationResponse::Processing)
    }

    fn handle_received_message<NT>(
        &mut self,
        _node: &mut InternalNode,
        _network: &NT,
        header: Header,
        message: OperationMessage,
    ) -> atlas_common::error::Result<OperationResponse> {
        let message = Self::from_operation_message(message);

        match message {
            QuorumObtainInfoOpMessage::RequestInformationMessage => {
                unreachable!("Received request information message while we are the ones requesting information")
            }
            QuorumObtainInfoOpMessage::QuorumInformationResponse(quorum) => match &mut self.state {
                OperationState::Waiting => {}
                OperationState::ReceivingInfo(received) if self.received.insert(header.from()) => {
                    debug!("Received quorum information from node {:?} with information {:?}, digest {:?}", header.from(), quorum, header.digest());
                    *received += 1;

                    let digest = *header.digest();

                    self.quorum_views
                        .entry(digest)
                        .or_default()
                        .push(StoredMessage::new(header, quorum));

                    if *received >= self.threshold {
                        if self
                            .quorum_views
                            .values()
                            .filter(|certs| certs.len() >= self.threshold)
                            .count()
                            > 0
                        {
                            info!("Received enough responses, and enough matching quorum views, finishing operation");

                            self.state = OperationState::Done;

                            return Ok(OperationResponse::Completed);
                        } else {
                            info!("Received enough responses, but not enough matching quorum views, waiting for more");
                        }
                    }
                }
                OperationState::ReceivingInfo(_) => {
                    error!("Received duplicate message from node {:?}", header.from());
                }
                OperationState::Done => debug!(
                    "Received message from node {:?} after operation was done",
                    header.from()
                ),
            },
        }

        Ok(OperationResponse::Processing)
    }

    fn handle_quorum_response<NT>(
        &mut self,
        _node: &mut InternalNode,
        _network: &NT,
        _quorum_response: QuorumReconfigurationResponse,
    ) -> atlas_common::error::Result<OperationResponse> {
        Ok(OperationResponse::Processing)
    }

    fn finish<NT>(
        self,
        observer: &mut InternalNode,
        _network: &NT,
    ) -> atlas_common::error::Result<()> {
        let view = self.finalize()?;

        observer.observer().install_quorum_view(view);

        observer.data_mut().insert(
            Self::OP_NAME,
            Self::LAST_EXEC,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        );

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum QuorumObtainInfoError {
    #[error("Failed to obtain quorum information: No messages received")]
    FailedNoMessagesReceived,
    #[error("Failed to obtain quorum information: Not enough matching messages, needed {0}, received {1}, digest {2:?}")]
    FailedNotEnoughMatching(usize, usize, Digest),
}
