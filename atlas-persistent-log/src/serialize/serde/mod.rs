use std::io::{Read, Write};
use atlas_common::error::*;
use atlas_core::ordering_protocol::{ProtocolMessage, SerProofMetadata, View};
use atlas_core::serialize::OrderingProtocolMessage;

pub(super) fn deserialize_message<R, OPM>(read: &mut R) -> Result<ProtocolMessage<OPM>>
    where R: Read,
          OPM: OrderingProtocolMessage {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize protocol message")
}

pub(super) fn deserialize_proof_metadata<R, OPM>(read: &mut R) -> Result<SerProofMetadata<OPM>>
    where R: Read,
          OPM: OrderingProtocolMessage {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize proof metadata")
}

pub(super) fn serialize_proof_metadata<W, OPM>(write: &mut W, proof: &SerProofMetadata<OPM>) -> Result<usize>
    where W: Write,
          OPM: OrderingProtocolMessage {
    bincode::serde::encode_into_std_write(proof, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize proof metadata")
}

pub(super) fn serialize_message<W, OPM>(write: &mut W, message: &ProtocolMessage<OPM>) -> Result<usize>
    where W: Write,
          OPM: OrderingProtocolMessage {
    bincode::serde::encode_into_std_write(message, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize message")
}

pub(super) fn serialize_view<W, OPM>(write: &mut W, view: &View<OPM>) -> Result<usize>
    where W: Write, OPM: OrderingProtocolMessage {
    bincode::serde::encode_into_std_write(view, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize view")
}

pub(super) fn deserialize_view<R, OPM>(read: &mut R) -> Result<View<OPM>>
    where R: Read, OPM: OrderingProtocolMessage {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize view")
}