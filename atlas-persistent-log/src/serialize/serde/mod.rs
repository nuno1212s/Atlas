use std::io::{Read, Write};
use atlas_common::error::*;
use atlas_core::persistent_log::{PersistableOrderProtocol, PSProofMetadata};
use crate::serialize::{PSMessage};

pub(super) fn deserialize_message<R, PS>(read: &mut R) -> Result<PSMessage<PS>> where R: Read, PS: PersistableOrderProtocol {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize protocol message")
}

pub(super) fn deserialize_proof_metadata<R, PS>(read: &mut R) -> Result<PSProofMetadata<PS>> where R: Read, PS:PersistableOrderProtocol {

    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize proof metadata")
}

pub(super) fn serialize_proof_metadata<W, PS>(write: &mut W, proof: &PSProofMetadata<PS>) -> Result<usize> where W: Write,
                                                                                                               PS: PersistableOrderProtocol {
    bincode::serde::encode_into_std_write(proof, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize proof metadata")
}

pub(super) fn serialize_message<W, PS>(write: &mut W, message: &PSMessage<PS>) -> Result<usize> where W: Write,
                                                                                                      PS: PersistableOrderProtocol {
    bincode::serde::encode_into_std_write(message, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize message")
}