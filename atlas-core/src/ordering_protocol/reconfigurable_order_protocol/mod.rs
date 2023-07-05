use atlas_common::error::*;
use atlas_reconfiguration::message::QuorumJoinCertificate;

pub enum ReconfigurationAttemptResult {
    Failed,
    InProgress,
    Successful,
}

/// The trait that defines the necessary operations for a given ordering protocol to be reconfigurable
pub trait ReconfigurableOrderProtocol {

    /// Attempt to finalize a network view change which has been requested by us.
    fn attempt_network_view_change(&mut self, join_certificate: QuorumJoinCertificate) -> Result<ReconfigurationAttemptResult>;



}