#[cfg(feature = "serialize_serde")]
mod serde;

#[cfg(feature = "serialize_capnp")]
mod capnp;

use std::io::{Read, Write};
use std::mem::size_of;
use ::serde::{Deserialize, Serialize};
use serde::{Deserialize, Serialize};
use atlas_capnp::objects_capnp;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::serialize::capnp;
use atlas_core::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};

pub(crate) type PSView<PS: PersistableStatefulOrderProtocol> = <PS::OrderProtocolMessage as OrderingProtocolMessage>::ViewInfo;

pub(crate) type PSMessage<PS: PersistableStatefulOrderProtocol> = <PS::OrderProtocolMessage as OrderingProtocolMessage>::ProtocolMessage;

/// A shortcut type to the proof type
pub(crate) type PSProof<PS: PersistableStatefulOrderProtocol> = <PS::StatefulOrderProtocolMessage as StatefulOrderProtocolMessage>::Proof;

pub(crate) type PSDecLog<PS: PersistableStatefulOrderProtocol> = <PS::StatefulOrderProtocolMessage as StatefulOrderProtocolMessage>::DecLog;

/// The trait with all the necessary types for the protocol to be used with our persistent storage system
/// We need this because of the way the messages are stored. Since we want to store the messages at the same
/// time we are receiving them, we divide the messages into various instances of KV-DB (which also parallelizes the
/// writing into them).
pub trait PersistableStatefulOrderProtocol {

    /// The type for the basic ordering protocol message
    type OrderProtocolMessage: OrderingProtocolMessage;

    /// The type of the stateful order protocol
    type StatefulOrderProtocolMessage: StatefulOrderProtocolMessage;

    /// The metadata type for storing the proof in the persistent storage
    /// Since the message will be stored in the persistent storage, it needs to be serializable
    /// This should provide all the necessary final information to assemble the proof from the messages
    #[cfg(feature = "serialize_capnp")]
    type ProofMetadata: Orderable + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type ProofMetadata: Orderable + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    /// The types of messages to be stored. This is used due to the parallelization described above.
    /// Each of the names provided here will be a different KV-DB instance (in the case of RocksDB, a column family)
    fn message_types() -> Vec<&'static str>;

    /// Get the message type for a given message, must correspond to a string returned by
    /// [PersistableStatefulOrderProtocol::message_types]
    fn get_type_for_message(msg: &PSMessage<Self>) -> Result<&str>;

    /// Initialize a proof from the metadata and messages stored in persistent storage
    fn init_proof_from(metadata: Self::ProofMetadata, messages: Vec<PSMessage<Self>>) -> Self::StatefulOrderProtocolMessage::Proof;

    /// Initialize a decision log from the messages stored in persistent storage
    fn init_dec_log(proofs: Vec<PSProof<Self>>) -> PSDecLog<Self>;

    /// Decompose a given proof into it's metadata and messages, ready to be persisted
    fn decompose_proof(proof: &PSProof<Self>) -> (&Self::ProofMetadata, Vec<&PSMessage<Self>>);

    /// Decompose a decision log into its separate proofs, so they can then be further decomposed
    /// into metadata and messages
    fn decompose_dec_log(proofs: &PSDecLog<Self>) -> Vec<&PSProof<Self>>;

    //TODO: capnp methods
}

pub(super) fn make_seq(seq: SeqNo) -> Result<Vec<u8>> {
    let mut seq_no = Vec::with_capacity(size_of::<SeqNo>());

    write_seq(&mut seq_no, seq)?;

    Ok(seq_no)
}


fn write_seq<W>(w: &mut W, seq: SeqNo) -> Result<()> where W: Write {
    let mut root = capnp::message::Builder::new(capnp::message::HeapAllocator::new());

    let mut seq_no: objects_capnp::seq::Builder = root.init_root();

    seq_no.set_seq_no(seq.into());

    capnp::serialize::write_message(w, &root).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize using capnp",
    )
}


pub(super) fn make_message_key(seq: SeqNo, from: Option<NodeId>) -> Result<Vec<u8>> {
    let mut key = Vec::with_capacity(size_of::<SeqNo>() + size_of::<NodeId>());

    write_message_key(&mut key, seq, from)?;

    Ok(key)
}

fn write_message_key<W>(w: &mut W, seq: SeqNo, from: Option<NodeId>) -> Result<()> where W: Write {
    let mut root = capnp::message::Builder::new(capnp::message::HeapAllocator::new());

    let mut msg_key: objects_capnp::message_key::Builder = root.init_root();

    let mut msg_seq_builder = msg_key.reborrow().init_msg_seq();

    msg_seq_builder.set_seq_no(seq.into());

    let mut msg_from = msg_key.reborrow().init_from();

    msg_from.set_node_id(from.unwrap_or(NodeId(0)).into());

    capnp::serialize::write_message(w, &root).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize using capnp",
    )
}

pub(super) fn read_seq<R>(r: R) -> Result<SeqNo> where R: Read {
    let reader = capnp::serialize::read_message(r, Default::default()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to get capnp reader",
    )?;

    let seq_no: objects_capnp::seq::Reader = reader.get_root().wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to get system msg root",
    )?;

    Ok(SeqNo::from(seq_no.get_seq_no()))
}

pub(super) fn serialize_message<W, PS>(w: &mut W, msg: &PSMessage<PS>) -> Result<usize> where W: Write, PS: PersistableStatefulOrderProtocol {

    #[cfg(feature="serialize_serde")]
    let res = serde::serialize_message(w, msg);

    #[cfg(feature="serialize_capnp")]
    todo!();

    res
}

pub(super) fn serialize_proof_metadata<W, PS>(w: &mut W, metadata: &PS::ProofMetadata) -> Result<usize> where W: Write, PS: PersistableStatefulOrderProtocol {

    #[cfg(feature="serialize_serde")]
    let res = serde::serialize_proof_metadata(w, metadata);

    #[cfg(feature="serialize_capnp")]
    todo!();

    res
}