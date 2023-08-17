use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use crate::ordering_protocol;
use crate::serialize::ReconfigurationProtocolMessage;

/// Result of attempting to change the quorum view
/// This should be returned immediately. In the case the quorum change
/// is not instantaneous, the ordering protocol should notify the replica of
/// the updated quorum as soon as the change is complete, utilizing the correspondent
/// [ordering_protocol::OrderProtocolExecResult] message
#[derive(Debug)]
pub enum ReconfigurationAttemptResult {
    Failed,
    AlreadyPartOfQuorum,
    CurrentlyReconfiguring(NodeId),
    InProgress,
    Successful,
}

/// The trait that defines the necessary operations for a given ordering protocol to be reconfigurable
/// Definitions:
/// - An ordering protocol can only accept one reconfiguration request at a given time (and should only support
/// one reconfiguration request at a time), Any other request during reconfiguration should return [ReconfigurationAttemptResult::CurrentlyReconfiguring]
/// or if there was another error, just return [ReconfigurationAttemptResult::Failed]. 
///
pub trait ReconfigurableOrderProtocol<RP> where RP: ReconfigurationProtocolMessage {

    /// Attempt to integrate a given node into the current view of the quorum.
    /// This function is only used when we are
    fn attempt_quorum_node_join(&mut self, joining_node: NodeId) -> Result<ReconfigurationAttemptResult>;

    /// We are, ourselves, attempting to join the quorum.
    fn joining_quorum(&mut self) -> Result<ReconfigurationAttemptResult>;

}