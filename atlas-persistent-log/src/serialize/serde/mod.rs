use std::io::{Read, Write};

use atlas_common::error::*;
use atlas_core::ordering_protocol::{LoggableMessage, SerProofMetadata, View};
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_execution::state::divisible_state::DivisibleState;
use atlas_execution::state::monolithic_state::MonolithicState;

pub(super) fn deserialize_message<R, D, OPM>(read: &mut R) -> Result<LoggableMessage<D, OPM>>
    where R: Read,
          OPM: OrderingProtocolMessage<D> {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize protocol message")
}

pub(super) fn deserialize_proof_metadata<R, D, OPM>(read: &mut R) -> Result<SerProofMetadata<D, OPM>>
    where R: Read,
          OPM: OrderingProtocolMessage<D> {
    bincode::serde::decode_from_std_read(read, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to deserialize proof metadata")
}

pub(super) fn serialize_proof_metadata<W, D, OPM>(write: &mut W, proof: &SerProofMetadata<D, OPM>) -> Result<usize>
    where W: Write,
          OPM: OrderingProtocolMessage<D> {
    bincode::serde::encode_into_std_write(proof, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize proof metadata")
}

pub(super) fn serialize_message<W, D, OPM>(write: &mut W, message: &LoggableMessage<D, OPM>) -> Result<usize>
    where W: Write,
          OPM: OrderingProtocolMessage<D> {
    bincode::serde::encode_into_std_write(message, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize message")
}

pub(super) fn serialize_view<W, D, OPM>(write: &mut W, view: &View<D, OPM>) -> Result<usize>
    where W: Write, OPM: OrderingProtocolMessage<D> {
    bincode::serde::encode_into_std_write(view, write, bincode::config::standard()).wrapped_msg(
        ErrorKind::MsgLogPersistentSerialization,
        "Failed to serialize view")
}

pub(super) fn deserialize_view<R, D, OPM>(read: &mut R) -> Result<View<D, OPM>>
    where R: Read, OPM: OrderingProtocolMessage<D> {
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