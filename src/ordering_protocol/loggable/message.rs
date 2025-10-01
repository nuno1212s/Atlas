use crate::ordering_protocol::networking::serialize::{
    OrderProtocolProof, OrderProtocolVerificationHelper, OrderingProtocolMessage,
};
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::reconfiguration::NetworkInformationProvider;
use std::sync::Arc;

/// The trait definining the necessary data types for the ordering protocol to be used
/// with the decision log
pub trait PersistentOrderProtocolTypes<RQ, OPM>: Send + Sync + 'static {
    /// A proof of a given Sequence number in the consensus protocol
    /// This is used as the type to fully represent the validity of a given SeqNo in the protocol
    /// A proof with SeqNo X should mean that X has been decided correctly
    /// This should be composed of some metadata and a set of LoggableMessages
    type Proof: OrderProtocolProof + SerMsg + 'static;

    /// Verify the validity of the given proof
    fn verify_proof<NI, OPVH>(
        network_info: &Arc<NI>,
        proof: Self::Proof,
    ) -> atlas_common::error::Result<Self::Proof>
    where
        NI: NetworkInformationProvider,
        OPM: OrderingProtocolMessage<RQ>,
        OPVH: OrderProtocolVerificationHelper<RQ, OPM, NI>,
        Self: Sized;
}
