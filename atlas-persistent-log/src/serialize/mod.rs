#[cfg(feature = "serialize_serde")]
mod serde;

#[cfg(feature = "serialize_capnp")]
mod capnp;

use std::io::{Read, Write};
use std::mem::size_of;
#[cfg(feature = "serialize_serde")]
use ::serde::{Deserialize, Serialize};
use atlas_capnp::objects_capnp;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_core::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};

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