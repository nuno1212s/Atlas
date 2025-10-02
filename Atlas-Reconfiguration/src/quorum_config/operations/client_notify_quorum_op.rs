use crate::message::OperationMessage;
use crate::quorum_config::network::QuorumConfigNetworkNode;
use crate::quorum_config::operations::{
    Operation, OperationExecutionCandidateError, OperationResponse,
};
use crate::quorum_config::{InternalNode, NodeStatusType};
use atlas_common::Err;
use atlas_communication::message::Header;
use atlas_core::reconfiguration_protocol::{QuorumReconfigurationResponse, QuorumUpdateMessage};

pub struct NotifyClientOperation {
    state: OperationState,
}

enum OperationState {
    Waiting,
    Done,
}

impl NotifyClientOperation {
    pub fn initialize() -> Self {
        Self {
            state: OperationState::Waiting,
        }
    }
}

impl Operation for NotifyClientOperation {
    const OP_NAME: &'static str = "NOTIFY_CLIENT";

    fn can_execute(observer: &InternalNode) -> Result<(), OperationExecutionCandidateError>
    where
        Self: Sized,
    {
        match observer.node_type() {
            NodeStatusType::ClientNode { .. } => Ok(()),
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
        match &self.state {
            OperationState::Waiting => {
                let view_members = node.observer().current_view().quorum_members().clone();

                node.node_type()
                    .client_communication()
                    .send(QuorumUpdateMessage::UpdatedQuorumView(view_members))?;

                self.state = OperationState::Done;

                Ok(OperationResponse::Completed)
            }
            OperationState::Done => Ok(OperationResponse::Completed),
        }
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
        _quorum_response: QuorumReconfigurationResponse,
    ) -> atlas_common::error::Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        Ok(OperationResponse::Processing)
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
