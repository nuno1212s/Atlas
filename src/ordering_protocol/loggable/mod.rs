pub mod message;

use atlas_common::error::*;
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::StoredMessage;

use crate::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use crate::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use crate::ordering_protocol::{
    DecisionAD, DecisionMetadata, OrderingProtocol, ProtocolConsensusDecision, ProtocolMessage,
    ShareableConsensusMessage,
};

/// The trait to define the necessary methods and data types for this order protocol
/// to be compatible with the decision log
pub trait LoggableOrderProtocol<RQ>:
    OrderingProtocol<RQ> + OrderProtocolLogHelper<RQ, Self::Serialization, Self::PersistableTypes>
where
    RQ: SerMsg,
{
    /// The required data types for working with the decision log
    type PersistableTypes: PersistentOrderProtocolTypes<RQ, Self::Serialization> + 'static;
}

pub trait OrderProtocolLogHelper<RQ, OP, PT>: 'static + Send
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
    PT: PersistentOrderProtocolTypes<RQ, OP>,
{
    /// The types of messages to be stored. This is used due to the parallelization described above.
    /// Each of the names provided here will be a different KV-DB instance (in the case of RocksDB, a column family)
    fn message_types() -> Vec<&'static str>;

    /// Get the message type for a given message, must correspond to a string returned by
    /// [PersistableOrderProtocol::message_types]
    fn get_type_for_message(msg: &ProtocolMessage<RQ, OP>) -> Result<&'static str>;

    /// Initialize a proof from the metadata and messages stored in persistent storage
    fn init_proof_from(
        metadata: DecisionMetadata<RQ, OP>,
        additional_data: Vec<DecisionAD<RQ, OP>>,
        messages: Vec<StoredMessage<ProtocolMessage<RQ, OP>>>,
    ) -> Result<PProof<RQ, OP, PT>>;

    /// Initialize a proof from the metadata and messages stored by the decision log
    fn init_proof_from_scm(
        metadata: DecisionMetadata<RQ, OP>,
        additional_data: Vec<DecisionAD<RQ, OP>>,
        messages: Vec<ShareableConsensusMessage<RQ, OP>>,
    ) -> Result<PProof<RQ, OP, PT>>;

    /// Decompose a given proof into it's metadata and messages, ready to be persisted
    fn decompose_proof(proof: &PProof<RQ, OP, PT>) -> DecomposedProof<RQ, OP>;

    /// Extract the proof out of the protocol decision proof
    fn get_requests_in_proof(proof: &PProof<RQ, OP, PT>) -> Result<ProtocolConsensusDecision<RQ>>;
}

pub type DecomposedProof<'a, RQ, OPM> = (
    &'a DecisionMetadata<RQ, OPM>,
    Vec<&'a DecisionAD<RQ, OPM>>,
    Vec<&'a StoredMessage<ProtocolMessage<RQ, OPM>>>,
);

pub type PProof<RQ, OP, POP> = <POP as PersistentOrderProtocolTypes<RQ, OP>>::Proof;
