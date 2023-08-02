use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use crate::reconfiguration_protocol::{QuorumJoinCert, ReconfigurationProtocol};
use crate::serialize::ReconfigurationProtocolMessage;

pub enum ReconfigurationAttemptResult {
    Failed,
    AlreadyPartOfQuorum,
    InProgress,
    Successful,
}

/// The trait that defines the necessary operations for a given ordering protocol to be reconfigurable
pub trait ReconfigurableOrderProtocol<RP> where RP: ReconfigurationProtocolMessage {

    /// Attempt to integrate a given node into the current view of the quorum.
    /// This function is only used when we are
    fn attempt_quorum_node_join(&mut self, joining_node: NodeId) -> Result<ReconfigurationAttemptResult>;

    /// We are, ourselves, attempting to join the quorum.
    fn joining_quorum(&mut self) -> Result<ReconfigurationAttemptResult>;

}