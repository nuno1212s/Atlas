use crate::message::OperationMessage;
use crate::quorum_config::network::QuorumConfigNetworkNode;
use crate::quorum_config::operations::{
    Operation, OperationExecutionCandidateError, OperationResponse,
};
use crate::quorum_config::InternalNode;
use atlas_common::node_id::NodeType;
use atlas_common::Err;
use atlas_communication::message::Header;
use atlas_core::reconfiguration_protocol::{
    QuorumReconfigurationMessage, QuorumReconfigurationResponse,
};

/// An operation made for when we first start up, perform our initial query of information
/// and discover that we are already a part of the current quorum.
/// Given this, we do not need to run [EnterQuorumOperation] and can instead just
/// directly contact the quorum and tell them we are ready to accept messages
pub struct NotifyQuorumOperation {
    current_state: OperationState,
}

pub enum OperationState {
    Waiting,
    WaitingQuorumResponse,
    Done,
}

impl NotifyQuorumOperation {
    pub fn initialize() -> Self {
        Self {
            current_state: OperationState::Waiting,
        }
    }
}

impl Operation for NotifyQuorumOperation {
    const OP_NAME: &'static str = "NOTIFY_QUORUM";

    fn can_execute(observer: &InternalNode) -> Result<(), OperationExecutionCandidateError>
    where
        Self: Sized,
    {
        match observer.node_info().node_type() {
            NodeType::Replica => Ok(()),
            _ => {
                Err!(OperationExecutionCandidateError::InvalidNodeType(
                    observer.node_info().node_type()
                ))
            }
        }
    }

    fn iterate<NT>(
        &mut self,
        node: &mut InternalNode,
        _network: &NT,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        if let OperationState::Waiting = self.current_state {
            let view_members = node.observer().current_view().quorum_members().clone();

            node.node_type().quorum_communication().send(
                QuorumReconfigurationMessage::ReconfigurationProtocolStable(view_members),
            )?;

            self.current_state = OperationState::WaitingQuorumResponse;

            return Ok(OperationResponse::AwaitingResponseProtocol);
        }

        Ok(OperationResponse::Processing)
    }

    fn handle_received_message<NT>(
        &mut self,
        _node: &mut InternalNode,
        _network: &NT,
        _header: Header,
        _message: OperationMessage,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        Ok(OperationResponse::Processing)
    }

    fn handle_quorum_response<NT>(
        &mut self,
        _node: &mut InternalNode,
        _network: &NT,
        quorum_response: QuorumReconfigurationResponse,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match self.current_state {
            OperationState::Waiting => {
                unreachable!("We should not be waiting for a quorum response at this point")
            }
            OperationState::WaitingQuorumResponse => match quorum_response {
                QuorumReconfigurationResponse::QuorumStableResponse(true) => {
                    self.current_state = OperationState::Done;

                    Ok(OperationResponse::Completed)
                }
                _ => Ok(OperationResponse::AwaitingResponseProtocol),
            },
            OperationState::Done => Ok(OperationResponse::Completed),
        }
    }

    fn finish<NT>(
        self,
        _observer: &mut InternalNode,
        _network: &NT,
    ) -> atlas_common::error::Result<()>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        Ok(())
    }
}
