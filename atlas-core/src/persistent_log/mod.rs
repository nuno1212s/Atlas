use std::path::Path;
use std::sync::Arc;
#[cfg(feature = "serialize_serde")]
use ::serde::{Deserialize, Serialize};
use atlas_execution::ExecutorHandle;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::StoredMessage;
use atlas_execution::serialize::SharedData;
use crate::ordering_protocol::{OrderingProtocol, ProtocolConsensusDecision, ProtocolMessage, SerProof, SerProofMetadata, View};
use crate::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage, StateTransferMessage};
use crate::state_transfer::{Checkpoint, DecLog, StatefulOrderProtocol, StateTransferProtocol};


///How should the data be written and response delivered?
/// If Sync is chosen the function will block on the call and return the result of the operation
/// If Async is chosen the function will not block and will return the response as a message to a channel
pub enum WriteMode {
    //When writing in async mode, you have the option of having the response delivered on a function
    //Of your choice
    //Note that this function will be executed on the persistent logging thread, so keep it short and
    //Be careful with race conditions.
    NonBlockingSync(Option<()>),
    BlockingSync,
}

/// Shortcuts for the types used in the protocol
/// The trait with all the necessary types for the protocol to be used with our persistent storage system
/// We need this because of the way the messages are stored. Since we want to store the messages at the same
/// time we are receiving them, we divide the messages into various instances of KV-DB (which also parallelizes the
/// writing into them).
pub trait PersistableOrderProtocol<OPM, SOPM> where OPM: OrderingProtocolMessage, SOPM: StatefulOrderProtocolMessage {

    /// The types of messages to be stored. This is used due to the parallelization described above.
    /// Each of the names provided here will be a different KV-DB instance (in the case of RocksDB, a column family)
    fn message_types() -> Vec<&'static str>;

    /// Get the message type for a given message, must correspond to a string returned by
    /// [PersistableOrderProtocol::message_types]
    fn get_type_for_message(msg: &ProtocolMessage<OPM>) -> Result<&'static str>;

    /// Initialize a proof from the metadata and messages stored in persistent storage
    fn init_proof_from(metadata: SerProofMetadata<OPM>, messages: Vec<StoredMessage<ProtocolMessage<OPM>>>) -> SerProof<OPM>;

    /// Initialize a decision log from the messages stored in persistent storage
    fn init_dec_log(proofs: Vec<SerProof<OPM>>) -> DecLog<SOPM>;

    /// Decompose a given proof into it's metadata and messages, ready to be persisted
    fn decompose_proof(proof: &SerProof<OPM>) -> (&SerProofMetadata<OPM>, Vec<&StoredMessage<ProtocolMessage<OPM>>>);

    /// Decompose a decision log into its separate proofs, so they can then be further decomposed
    /// into metadata and messages
    fn decompose_dec_log(proofs: &DecLog<SOPM>) -> Vec<&SerProof<OPM>>;
}

pub trait PersistableStateTransferProtocol {



}

/// The trait necessary for a logging protocol capable of simple (stateless) ordering.
/// Does not have any methods for proofs or decided logs since in theory there is no need for them
pub trait OrderingProtocolLog<OP>: Clone where OP: OrderingProtocolMessage {
    /// Write to the persistent log the latest committed sequence number
    fn write_committed_seq_no(&self, write_mode: WriteMode, seq: SeqNo) -> Result<()>;

    /// Write to the persistent log the latest View information
    fn write_view_info(&self, write_mode: WriteMode, view_seq: View<OP>) -> Result<()>;

    /// Write a given message to the persistent log
    fn write_message(&self, write_mode: WriteMode, msg: Arc<ReadOnly<StoredMessage<ProtocolMessage<OP>>>>) -> Result<()>;

    /// Write the metadata for a given proof to the persistent log
    /// This in combination with the messages for that sequence number should form a valid proof
    fn write_proof_metadata(&self, write_mode: WriteMode, metadata: SerProofMetadata<OP>) -> Result<()>;

    /// Write a given proof to the persistent log
    fn write_proof(&self, write_mode: WriteMode, proof: SerProof<OP>) -> Result<()>;

    /// Invalidate all messages with sequence number equal to the given one
    fn write_invalidate(&self, write_mode: WriteMode, seq: SeqNo) -> Result<()>;
}

/// Complements the default [`OrderingProtocolLog`] with methods for proofs and decided logs
pub trait StatefulOrderingProtocolLog<OPM, SOPM>: OrderingProtocolLog<OPM>
    where OPM: OrderingProtocolMessage, SOPM: StatefulOrderProtocolMessage {
    fn read_state(&self, write_mode: WriteMode) -> Result<Option<(View<OPM>, DecLog<SOPM>)>>;

    /// Write a given decision log to the persistent log
    fn write_install_state(&self, write_mode: WriteMode, view: View<OPM>, dec_log: DecLog<SOPM>) -> Result<()>;
}

pub trait StateTransferProtocolLog<OPM, SOPM, D>: StatefulOrderingProtocolLog<OPM, SOPM>
    where OPM: OrderingProtocolMessage, SOPM: StatefulOrderProtocolMessage, D: SharedData {
    /// Write a checkpoint to the persistent log
    fn write_checkpoint(
        &self,
        write_mode: WriteMode,
        checkpoint: Arc<ReadOnly<Checkpoint<D::State>>>,
    ) -> Result<()>;
}