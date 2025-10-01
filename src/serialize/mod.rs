use std::fmt::Debug;
use std::sync::Arc;

#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration::NetworkInformationProvider;
use atlas_communication::serialization::{InternalMessageVerifier, Serializable};

use crate::ordering_protocol::networking::serialize::{NetworkView, OrderProtocolProof};

#[cfg(feature = "serialize_capnp")]
pub mod capnp;

/// Reconfiguration protocol messages
pub trait ReconfigurationProtocolMessage: Serializable + Send + Sync {
    type QuorumJoinCertificate: SerMsg;
}

#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Debug)]
pub struct NoProtocol;

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct NoView;

impl Orderable for NoView {
    fn sequence_number(&self) -> SeqNo {
        unimplemented!()
    }
}

impl NetworkView for NoView {
    fn primary(&self) -> NodeId {
        unimplemented!()
    }

    fn quorum(&self) -> usize {
        unimplemented!()
    }

    fn quorum_members(&self) -> &Vec<NodeId> {
        unimplemented!()
    }

    fn f(&self) -> usize {
        unimplemented!()
    }

    fn n(&self) -> usize {
        unimplemented!()
    }
}

impl Serializable for NoProtocol {
    type Message = ();
    type Verifier = Self;
}

impl InternalMessageVerifier<()> for NoProtocol {
    fn verify_message<NI>(
        _info_provider: &Arc<NI>,
        _header: &Header,
        _message: &(),
    ) -> atlas_common::error::Result<()>
    where
        NI: NetworkInformationProvider,
    {
        Ok(())
    }
}

impl OrderProtocolProof for () {
    fn contained_messages(&self) -> usize {
        0
    }
}
