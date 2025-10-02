use thiserror::Error;

use atlas_common::error::*;
use atlas_common::node_id;
use atlas_communication::message::Header;
use atlas_core::reconfiguration_protocol::QuorumReconfigurationResponse;

use crate::message::OperationMessage;
use crate::quorum_config::network::QuorumConfigNetworkNode;
use crate::quorum_config::operations::client_notify_quorum_op::NotifyClientOperation;
use crate::quorum_config::operations::notify_stable_quorum::NotifyQuorumOperation;
use crate::quorum_config::operations::quorum_accept_op::QuorumAcceptNodeOperation;
use crate::quorum_config::operations::quorum_info_op::ObtainQuorumInfoOP;
use crate::quorum_config::operations::quorum_join_op::EnterQuorumOperation;
use crate::quorum_config::InternalNode;

pub(crate) mod client_notify_quorum_op;
pub(crate) mod notify_stable_quorum;
pub(crate) mod propagate_quorum_info;
pub(crate) mod quorum_accept_op;
pub(crate) mod quorum_info_op;
pub(crate) mod quorum_join_op;

/// The operation object
pub enum OperationObj {
    QuorumInfoOp(ObtainQuorumInfoOP),
    QuorumJoinOp(EnterQuorumOperation),
    QuorumAcceptOp(QuorumAcceptNodeOperation),
    NotifyQuorumOp(NotifyQuorumOperation),
    NotifyClientOp(NotifyClientOperation),
}

#[derive(Clone)]
/// The various responses the operation can return
pub enum OperationResponse {
    /// The operation has been completed, and the replica can move on to the next one
    Completed,
    CompletedFailed,
    /// The operation is still in progress, and the replica should wait for the next iteration
    Processing,
    /// We have issued a quorum reconfiguration request and are waiting for the response
    AwaitingResponseProtocol,
    NoLongerAwaitingResponseProtocol,
}

/// The trait that represents a possible operation on the current state of the quorum
pub trait Operation {
    const OP_NAME: &'static str;

    fn op_name() -> &'static str
    where
        Self: Sized,
    {
        Self::OP_NAME
    }

    /// Can a given node execute this type of operation
    fn can_execute(
        observer: &InternalNode,
    ) -> std::result::Result<(), OperationExecutionCandidateError>
    where
        Self: Sized;

    /// We want to iterate through this operation (for example to launch the initial requests,
    /// perform re requests on timeouts, etc)
    fn iterate<NT>(&mut self, node: &mut InternalNode, network: &NT) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static;

    /// Handle a message that we have received from another node in the context of this operation
    fn handle_received_message<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        header: Header,
        message: OperationMessage,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static;

    /// Handle the response of a quorum reconfiguration request sent by this operation
    fn handle_quorum_response<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        quorum_response: QuorumReconfigurationResponse,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static;

    /// Finish an operation by passing it the node so it can perform the necessary changes
    fn finish<NT>(self, observer: &mut InternalNode, network: &NT) -> Result<()>
    where
        NT: QuorumConfigNetworkNode + 'static;
}

impl OperationObj {
    pub fn op_name(&self) -> &'static str {
        match self {
            OperationObj::QuorumInfoOp(_) => ObtainQuorumInfoOP::op_name(),
            OperationObj::QuorumJoinOp(_) => EnterQuorumOperation::op_name(),
            OperationObj::QuorumAcceptOp(_) => QuorumAcceptNodeOperation::op_name(),
            OperationObj::NotifyQuorumOp(_) => NotifyQuorumOperation::op_name(),
            OperationObj::NotifyClientOp(_) => NotifyClientOperation::op_name(),
        }
    }

    pub fn can_execute(
        &self,
        observer: &InternalNode,
    ) -> std::result::Result<(), OperationExecutionCandidateError> {
        match self {
            OperationObj::QuorumInfoOp(_) => ObtainQuorumInfoOP::can_execute(observer),
            OperationObj::QuorumJoinOp(_) => EnterQuorumOperation::can_execute(observer),
            OperationObj::QuorumAcceptOp(_) => QuorumAcceptNodeOperation::can_execute(observer),
            OperationObj::NotifyQuorumOp(_) => NotifyQuorumOperation::can_execute(observer),
            OperationObj::NotifyClientOp(_) => NotifyClientOperation::can_execute(observer),
        }
    }

    pub fn iterate<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match self {
            OperationObj::QuorumInfoOp(op) => op.iterate(node, network),
            OperationObj::QuorumJoinOp(op) => op.iterate(node, network),
            OperationObj::QuorumAcceptOp(op) => op.iterate(node, network),
            OperationObj::NotifyQuorumOp(op) => op.iterate(node, network),
            OperationObj::NotifyClientOp(op) => op.iterate(node, network),
        }
    }

    pub fn handle_received_message<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        header: Header,
        message: OperationMessage,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match self {
            OperationObj::QuorumInfoOp(op) => {
                op.handle_received_message(node, network, header, message)
            }
            OperationObj::QuorumJoinOp(op) => {
                op.handle_received_message(node, network, header, message)
            }
            OperationObj::QuorumAcceptOp(op) => {
                op.handle_received_message(node, network, header, message)
            }
            OperationObj::NotifyQuorumOp(op) => {
                op.handle_received_message(node, network, header, message)
            }
            OperationObj::NotifyClientOp(op) => {
                op.handle_received_message(node, network, header, message)
            }
        }
    }

    pub fn handle_quorum_response<NT>(
        &mut self,
        node: &mut InternalNode,
        network: &NT,
        quorum_response: QuorumReconfigurationResponse,
    ) -> Result<OperationResponse>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match self {
            OperationObj::QuorumInfoOp(op) => {
                op.handle_quorum_response(node, network, quorum_response)
            }
            OperationObj::QuorumJoinOp(op) => {
                op.handle_quorum_response(node, network, quorum_response)
            }
            OperationObj::QuorumAcceptOp(op) => {
                op.handle_quorum_response(node, network, quorum_response)
            }
            OperationObj::NotifyQuorumOp(op) => {
                op.handle_quorum_response(node, network, quorum_response)
            }
            OperationObj::NotifyClientOp(op) => {
                op.handle_quorum_response(node, network, quorum_response)
            }
        }
    }

    pub fn finish<NT>(self, observer: &mut InternalNode, network: &NT) -> Result<()>
    where
        NT: QuorumConfigNetworkNode + 'static,
    {
        match self {
            OperationObj::QuorumInfoOp(op) => op.finish(observer, network),
            OperationObj::QuorumJoinOp(op) => op.finish(observer, network),
            OperationObj::QuorumAcceptOp(op) => op.finish(observer, network),
            OperationObj::NotifyQuorumOp(op) => op.finish(observer, network),
            OperationObj::NotifyClientOp(op) => op.finish(observer, network),
        }
    }
}

/// Errors that describe why a given quorum observer cannot execute a given operation
#[derive(Error, Debug)]
pub enum OperationExecutionCandidateError {
    #[error("This operation cannot be executed by a {0:?}")]
    InvalidNodeType(node_id::NodeType),
    #[error("Another operation which conflicts with this one is already ongoing")]
    ConflictingOperations,
    #[error("The requirements for this operation are not met {0}")]
    RequirementsNotMet(String),
}
