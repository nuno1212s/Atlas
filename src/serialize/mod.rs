#[cfg(feature = "serialize_serde")]
mod serde;

#[cfg(feature = "serialize_capnp")]
mod _capnp;

use anyhow::Context;
use atlas_capnp::objects_capnp;
use atlas_common::error::Result;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_common::serialization_helper::SerMsg;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::networking::serialize::{
    OrderingProtocolMessage, PermissionedOrderingProtocolMessage,
};
use atlas_core::ordering_protocol::{DecisionMetadata, ProtocolMessage, View};
use atlas_logging_core::decision_log::serialize::DecisionLogMessage;
use atlas_logging_core::decision_log::DecLogMetadata;
use atlas_smr_application::state::divisible_state::DivisibleState;
use atlas_smr_application::state::monolithic_state::MonolithicState;
use std::io::{Read, Write};
use std::mem::size_of;

pub(super) fn make_seq(seq: SeqNo) -> Result<Vec<u8>> {
    let mut seq_no = Vec::with_capacity(size_of::<SeqNo>());

    write_seq(&mut seq_no, seq)?;

    Ok(seq_no)
}

fn write_seq<W>(w: &mut W, seq: SeqNo) -> Result<()>
where
    W: Write,
{
    let mut root = capnp::message::Builder::new(capnp::message::HeapAllocator::new());

    let mut seq_no: objects_capnp::seq::Builder = root.init_root();

    seq_no.set_seq_no(seq.into());

    capnp::serialize::write_message(w, &root).context("Failed to write sequence number into writer")
}

pub(super) fn make_message_key(seq: SeqNo, from: Option<NodeId>) -> Result<Vec<u8>> {
    let mut key = Vec::with_capacity(size_of::<SeqNo>() + size_of::<NodeId>());

    write_message_key(&mut key, seq, from)?;

    Ok(key)
}

fn write_message_key<W>(w: &mut W, seq: SeqNo, from: Option<NodeId>) -> Result<()>
where
    W: Write,
{
    let mut root = capnp::message::Builder::new(capnp::message::HeapAllocator::new());

    let mut msg_key: objects_capnp::message_key::Builder = root.init_root();

    let mut msg_seq_builder = msg_key.reborrow().init_msg_seq();

    msg_seq_builder.set_seq_no(seq.into());

    let mut msg_from = msg_key.reborrow().init_from();

    msg_from.set_node_id(from.unwrap_or(NodeId(0)).into());

    capnp::serialize::write_message(w, &root).context("Failed to serialize message key into writer")
}

pub(super) fn read_seq<R>(r: R) -> Result<SeqNo>
where
    R: Read,
{
    let reader = capnp::serialize::read_message(r, Default::default())
        .context("Failed to read sequence message from reader")?;

    let seq_no: objects_capnp::seq::Reader = reader
        .get_root()
        .context("Failed to parse sequence number from capnp reader")?;

    Ok(SeqNo::from(seq_no.get_seq_no()))
}

pub(super) fn serialize_view<W, POP>(w: &mut W, view: &View<POP>) -> Result<usize>
where
    W: Write,
    POP: PermissionedOrderingProtocolMessage,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::serialize_view::<W, POP>(w, view);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn serialize_message<W, RQ, OPM>(
    w: &mut W,
    msg: &ProtocolMessage<RQ, OPM>,
) -> Result<usize>
where
    W: Write,
    OPM: OrderingProtocolMessage<RQ>,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::serialize_message::<W, RQ, OPM>(w, msg);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn serialize_proof_metadata<W, D, OPM>(
    w: &mut W,
    metadata: &DecisionMetadata<D, OPM>,
) -> Result<usize>
where
    W: Write,
    OPM: OrderingProtocolMessage<D>,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::serialize_proof_metadata::<W, D, OPM>(w, metadata);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn serialize_decision_log_metadata<W, RQ, OPM, POPT, LS>(
    w: &mut W,
    metadata: &DecLogMetadata<RQ, OPM, POPT, LS>,
) -> Result<usize>
where
    W: Write,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::serialize_decision_log_metadata::<W, RQ, OPM, POPT, LS>(w, metadata);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn deserialize_view<R, POP>(r: &mut R) -> Result<View<POP>>
where
    R: Read,
    POP: PermissionedOrderingProtocolMessage,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::deserialize_view::<R, POP>(r);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn deserialize_message<R, D, OPM>(r: &mut R) -> Result<ProtocolMessage<D, OPM>>
where
    R: Read,
    OPM: OrderingProtocolMessage<D>,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::deserialize_message::<R, D, OPM>(r);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn deserialize_decision_log_metadata<R, RQ, OPM, POPT, LS>(
    r: &mut R,
) -> Result<DecLogMetadata<RQ, OPM, POPT, LS>>
where
    R: Read,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::deserialize_decision_log_metadata::<R, RQ, OPM, POPT, LS>(r);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn deserialize_proof_metadata<R, RQ, OPM>(r: &mut R) -> Result<DecisionMetadata<RQ, OPM>>
where
    R: Read,
    OPM: OrderingProtocolMessage<RQ>,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::deserialize_proof_metadata::<R, RQ, OPM>(r);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn serialize_mon_state<W, S>(w: &mut W, state: &S) -> Result<usize>
where
    W: Write,
    S: MonolithicState,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::serialize_state::<W, S>(w, state);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn deserialize_mon_state<R, S>(r: &mut R) -> Result<S>
where
    R: Read,
    S: MonolithicState,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::deserialize_state::<R, S>(r);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn serialize_state_part_descriptor<W, S>(
    w: &mut W,
    state_desc: &S::PartDescription,
) -> Result<usize>
where
    W: Write,
    S: DivisibleState,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::serialize_state_part_descriptor::<W, S>(w, state_desc);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn deserialize_state_part_descriptor<R, S>(r: &mut R) -> Result<S::PartDescription>
where
    R: Read,
    S: DivisibleState,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::deserialize_state_part_descriptor::<R, S>(r);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn serialize_state_part<W, S>(w: &mut W, state_parts: &S::StatePart) -> Result<usize>
where
    W: Write,
    S: DivisibleState,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::serialize_state_part::<W, S>(w, state_parts);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn deserialize_state_part<R, S>(r: &mut R) -> Result<S::StatePart>
where
    R: Read,
    S: DivisibleState,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::deserialize_state_part::<R, S>(r);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn serialize_state_descriptor<W, S>(
    w: &mut W,
    descriptor: &S::StateDescriptor,
) -> Result<usize>
where
    W: Write,
    S: DivisibleState,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::serialize_state_descriptor::<W, S>(w, descriptor);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}

pub(super) fn deserialize_state_descriptor<R, S>(r: &mut R) -> Result<S::StateDescriptor>
where
    R: Read,
    S: DivisibleState,
{
    #[cfg(feature = "serialize_serde")]
    let res = serde::deserialize_state_descriptor::<R, S>(r);

    #[cfg(feature = "serialize_capnp")]
    let res = todo!();

    res
}
