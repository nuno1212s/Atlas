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
use crate::ordering_protocol::ProtocolConsensusDecision;
use crate::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};
use crate::state_transfer::Checkpoint;


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

pub type PSView<PS: PersistableOrderProtocol> = <PS::OrderProtocolMessage as OrderingProtocolMessage>::ViewInfo;

pub type PSMessage<PS: PersistableOrderProtocol> = <PS::OrderProtocolMessage as OrderingProtocolMessage>::ProtocolMessage;

/// A shortcut type to the proof type
pub type PSProof<PS: PersistableOrderProtocol> = <PS::StatefulOrderProtocolMessage as StatefulOrderProtocolMessage>::Proof;

pub type PSDecLog<PS: PersistableOrderProtocol> = <PS::StatefulOrderProtocolMessage as StatefulOrderProtocolMessage>::DecLog;

/// The trait with all the necessary types for the protocol to be used with our persistent storage system
/// We need this because of the way the messages are stored. Since we want to store the messages at the same
/// time we are receiving them, we divide the messages into various instances of KV-DB (which also parallelizes the
/// writing into them).
pub trait PersistableOrderProtocol {

    /// The type for the basic ordering protocol message
    type OrderProtocolMessage: OrderingProtocolMessage;

    /// The type of the stateful order protocol
    type StatefulOrderProtocolMessage: StatefulOrderProtocolMessage;

    /// The metadata type for storing the proof in the persistent storage
    /// Since the message will be stored in the persistent storage, it needs to be serializable
    /// This should provide all the necessary final information to assemble the proof from the messages
    #[cfg(feature = "serialize_serde")]
    type ProofMetadata: Orderable + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type ProofMetadata: Orderable + Send + Clone;

    /// The types of messages to be stored. This is used due to the parallelization described above.
    /// Each of the names provided here will be a different KV-DB instance (in the case of RocksDB, a column family)
    fn message_types() -> Vec<&'static str>;

    /// Get the message type for a given message, must correspond to a string returned by
    /// [PersistableOrderProtocol::message_types]
    fn get_type_for_message(msg: &PSMessage<Self>) -> Result<&str>;

    /// Initialize a proof from the metadata and messages stored in persistent storage
    fn init_proof_from(metadata: Self::ProofMetadata, messages: Vec<PSMessage<Self>>) -> PSProof<Self>;

    /// Initialize a decision log from the messages stored in persistent storage
    fn init_dec_log(proofs: Vec<PSProof<Self>>) -> PSDecLog<Self>;

    /// Decompose a given proof into it's metadata and messages, ready to be persisted
    fn decompose_proof(proof: &PSProof<Self>) -> (&Self::ProofMetadata, Vec<&PSMessage<Self>>);

    /// Decompose a decision log into its separate proofs, so they can then be further decomposed
    /// into metadata and messages
    fn decompose_dec_log(proofs: &PSDecLog<Self>) -> Vec<&PSProof<Self>>;

    //TODO: capnp methods
}

pub trait PersistableStateTransferProtocol {

    /// The metadata type for storing the proof in the persistent storage
    /// Since the message will be stored in the persistent storage, it needs to be serializable
    /// This should provide all the necessary final information to assemble the proof from the messages
    #[cfg(feature = "serialize_serde")]
    type State: Orderable + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type State: Orderable + Send + Clone;

}

pub trait OrderingProtocolPersistentLog<PS> where PS: PersistableOrderProtocol {

    fn write_committed_seq_no(&self, write_mode: WriteMode, seq: SeqNo) -> Result<()>;

    fn write_view_info(&self, write_mode: WriteMode, view_seq: PSView<PS>) -> Result<()>;

    fn write_proof_metadata(&self, write_mode: WriteMode,
                            metadata: PS::ProofMetadata) -> Result<()>;

    fn write_message(
        &self,
        write_mode: WriteMode,
        msg: Arc<ReadOnly<StoredMessage<PSMessage<PS>>>>,
    ) -> Result<()>;

    fn write_proof(&self, write_mode: WriteMode, proof: PSProof<PS>) -> Result<()>;

    fn write_invalidate(&self, write_mode: WriteMode, seq: SeqNo) -> Result<()>;
}

pub trait StateTransferPersistentLog<PS> where PS: PersistableStateTransferProtocol {

    fn write_checkpoint(
        &self,
        write_mode: WriteMode,
        checkpoint: Arc<ReadOnly<Checkpoint<PS::State>>>,
    ) -> Result<()>;

}