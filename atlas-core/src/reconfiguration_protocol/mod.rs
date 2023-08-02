use std::sync::Arc;

use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_communication::reconfiguration_node::{NetworkInformationProvider, ReconfigurationNode};

use crate::serialize::ReconfigurationProtocolMessage;
use crate::timeouts::{RqTimeout, Timeouts};

/// Messages to be sent by the reconfiguration protocol
/// to the ordering protocol relating changes that have undergone in the
/// Quorum View.
pub enum QuorumReconfigurationMessage {
    /// The reconfiguration protocol has reached stability and we can now start to execute the
    /// Quorum protocol, with the given base nodes
    ReconfigurationProtocolStable(Vec<NodeId>),
    // We have been granted permission into an existing quorum, and we must
    // now indicate to the ordering protocol that he can attempt to join the quorum
    RequestQuorumJoin(NodeId),
    // We are going to attempt to join the quorum
    AttemptToJoinQuorum
}

/// Messages sent by the ordering protocol to notify the reconfiguration protocol of changes
/// to the quorum
pub enum QuorumReconfigurationResponse {
    QuorumAlterationResponse(QuorumAlterationResponse),
    QuorumUpdate(QuorumUpdateMessage)
}

pub enum QuorumAlterationResponse {
    Successful,
    Failed(),
}

/// Analogous to the `QuorumReconfigurationMessage`, this is the message that the ordering protocol
/// will send to the reconfiguration protocol to notify it of changes in the quorum view
/// This is aimed for clients, which only listen to quorum updates, they don't actually participate
pub enum QuorumUpdateMessage {
    UpdatedQuorumView(Vec<NodeId>),
}

pub enum ReconfigurableNodeTypes {
    Client(ChannelSyncTx<QuorumUpdateMessage>),
    Replica(ChannelSyncTx<QuorumReconfigurationMessage>,
            ChannelSyncRx<QuorumReconfigurationResponse>),
}

pub type QuorumJoinCert<RP: ReconfigurationProtocolMessage> = RP::QuorumJoinCertificate;

pub enum ReconfigResponse {
    Running,
    Stop,
}

/// The trait defining the necessary functionality for a reconfiguration protocol (at least at the moment)
///
/// This is different from the other protocols like ordering, state and log transfer since we actually
/// Run this protocol independently from the rest of the system, only sending and receiving updates
/// through message passing (unlike the state and log transfer, which run in the same thread since
/// we cannot execute the ordering protocol while we are executing log and state transfers)
pub trait ReconfigurationProtocol: Send + Sync + 'static {
    // The configuration type the protocol wants to receive
    type Config;

    /// Type of the information provider that the protocol will provide
    type InformationProvider: NetworkInformationProvider;

    /// Type of the message that the protocol will use, to be used by the networking layer
    type Serialization: ReconfigurationProtocolMessage + 'static;

    /// Initialize a default information object from the provided configuration.
    /// This object will be used to initialize the networking protocol.
    fn init_default_information(config: Self::Config) -> Result<Arc<Self::InformationProvider>>;

    /// After initializing the networking protocol with the necessary information provider,
    /// we can then start to initialize the reconfiguration protocol. At the moment, differently from
    /// the ordering, state transfer and log transfer protocols, the reconfiguration protocol
    /// is meant to run completely independently from the rest of the system, only sending and receiving
    /// updates
    async fn initialize_protocol<NT>(information: Arc<Self::InformationProvider>,
                                     node: Arc<NT>, timeouts: Timeouts,
                                     node_type: ReconfigurableNodeTypes,
                                     min_stable_node_count: usize) -> Result<Self>
        where NT: ReconfigurationNode<Self::Serialization> + 'static, Self: Sized;

    /// Handle a timeout from the timeouts layer
    fn handle_timeout(&self, timeouts: Vec<RqTimeout>) -> Result<ReconfigResponse>;

    /// Get the current quorum members of the system
    fn get_quorum_members(&self) -> Vec<NodeId>;

    /// Check if a given join certificate is valid
    fn is_join_certificate_valid(&self, certificate: &QuorumJoinCert<Self::Serialization>) -> bool;
}