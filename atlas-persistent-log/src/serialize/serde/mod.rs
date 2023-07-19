use std::io::{Read, Write};
use atlas_common::error::*;
use atlas_core::ordering_protocol::{LoggableMessage, ProtocolMessage, SerProofMetadata, View};
use atlas_core::serialize::OrderingProtocolMessage;
use atlas_execution::state::divisible_state::DivisibleState;
use atlas_execution::state::monolithic_state::MonolithicState;

pub(super) fn deserialize_message<R, OPM>(read: &mut R) -> Result<LoggableMessage<OPM>>
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

pub(super) fn serialize_message<W, OPM>(write: &mut W, message: &LoggableMessage<OPM>) -> Result<usize>
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

pub(super) fn serialize_state_part_descriptor<W, S>(write: &mut W, part: &S::PartDescription) -> Result<usize>
    where W: Write, S: DivisibleState {
    bincode::serde::encode_into_std_write(&part, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize state part descriptor")
}

pub(super) fn deserialize_state_part_descriptor<R, S>(read: &mut R) -> Result<S::PartDescription>
    where R: Read, S: DivisibleState {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize state part descriptor")
}

pub(super) fn serialize_state_part<W, S>(write: &mut W, part: &S::StatePart) -> Result<usize>
    where W: Write, S: DivisibleState {
    bincode::serde::encode_into_std_write(&part, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize state part")
}

pub(super) fn deserialize_state_part<R, S>(read: &mut R) -> Result<S::StatePart>
    where R: Read, S: DivisibleState {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize state part")
}

pub(super) fn serialize_state_descriptor<W, S>(write: &mut W, desc: &S::StateDescriptor) -> Result<usize>
    where W: Write, S: DivisibleState {
    bincode::serde::encode_into_std_write(&desc, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize state descriptor")
}

pub(super) fn deserialize_state_descriptor<R, S>(read: &mut R) -> Result<S::StateDescriptor>
    where R: Read, S: DivisibleState {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize state descriptor")
}

pub(super) fn serialize_state<W, S>(write: &mut W, state: &S) -> Result<usize>
    where W: Write, S: MonolithicState {
    bincode::serde::encode_into_std_write(&state, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize state")
}

pub(super) fn deserialize_state<R, S>(read: &mut R) -> Result<S>
    where R: Read, S: MonolithicState {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize state")
}