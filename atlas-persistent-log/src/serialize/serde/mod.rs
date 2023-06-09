use std::io::Write;
use atlas_common::error::*;
use crate::serialize::{PersistableStatefulOrderProtocol, PSMessage};

pub(super) fn serialize_proof_metadata<W, PS>(write: &mut W, proof: &PS::ProofMetadata) -> Result<usize> where W: Write,
PS: PersistableStatefulOrderProtocol {

    bincode::serde::encode_into_std_write(proof, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize proof metadata")

}

pub(super) fn serialize_message<W, PS>(write: &mut W, message: &PSMessage<PS>) -> Result<usize> where W: Write,
PS: PersistableStatefulOrderProtocol {

    bincode::serde::encode_into_std_write(message, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize message")

}