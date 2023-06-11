use std::io::{Read, Write};
use atlas_common::error::*;
use atlas_core::persistent_log::PersistableOrderProtocol;
use crate::serialize::{PersistableStatefulOrderProtocol, PSMessage};

pub(super) fn deserialize_message<R, PS>(read: R) -> Result<PSMessage<PS>> where R: Read, PS: PersistableOrderProtocol {
    bincode::serde::decode_from_reader(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize protocol message")
}

pub(super) fn deserialize_proof_metadata<R, PS>(read: R) -> Result<PS::ProofMetadata> where R: Read, PS:PersistableOrderProtocol {

    bincode::serde::decode_from_reader(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize proof metadata")
}

pub(super) fn serialize_proof_metadata<W, PS>(write: &mut W, proof: &PS::ProofMetadata) -> Result<usize> where W: Write,
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