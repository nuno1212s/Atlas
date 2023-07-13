use std::sync::Arc;
use atlas_common::error::*;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::node_id::NodeId;
use atlas_communication::reconfiguration_node::{NetworkInformationProvider, ReconfigurationNode};
use atlas_communication::serialize::Serializable;
use crate::serialize::ReconfigurationProtocolMessage;

pub enum QuorumReconfigurationMessage<JC>  {
    RequestQuorumViewAlteration(JC),
}

pub enum QuorumReconfigurationResponse {
    QuorumAlterationResponse(QuorumAlterationResponse),
}

pub enum QuorumAlterationResponse {
    Successful,
    Failed(),
}

pub enum ReconfigurableNodeTypes<JC> {
    Client,
    Replica(ChannelSyncTx<QuorumReconfigurationMessage<JC>>,
            ChannelSyncRx<QuorumReconfigurationResponse>),
}

pub type QuorumJoinCert<RP: ReconfigurationProtocolMessage> = RP::QuorumJoinCertificate;

/// The trait defining the necessary functionality for a reconfiguration protocol (at least at the moment)
pub trait ReconfigurationProtocol<NT>: Send + Sync + 'static {

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
    fn initialize_protocol(information: Arc<Self::InformationProvider>, node: Arc<NT>, node_type: ReconfigurableNodeTypes<QuorumJoinCert<Self::Serialization>>) -> Result<Self>
        where NT: ReconfigurationNode<Self::Serialization> + 'static, Self: Sized;

    /// Get the current quorum members of the system
    fn get_quorum_members(&self) -> Vec<NodeId>;

    /// Check if a given join certificate is valid
    fn is_join_certificate_valid(&self, certificate: &QuorumJoinCert<Self::Serialization>) -> bool;

}