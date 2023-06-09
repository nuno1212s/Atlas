use std::sync::Arc;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::persistentdb::KVDB;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_communication::message::{Header, StoredMessage};
use atlas_core::state_transfer::Checkpoint;
use atlas_execution::serialize::SharedData;
use crate::{InstallState, serialize};
use crate::serialize::{PersistableStatefulOrderProtocol, PSDecLog, PSMessage, PSProof};

///Latest checkpoint made by the execution
const LATEST_STATE: &str = "latest_state";

///First sequence number (committed) since the last checkpoint
const FIRST_SEQ: &str = "first_seq";
///Last sequence number (committed) since the last checkpoint
const LATEST_SEQ: &str = "latest_seq";
///Latest known view sequence number
const LATEST_VIEW_SEQ: &str = "latest_view_seq";

/// The default column family for the persistent logging
const COLUMN_FAMILY_OTHER: &str = "other";
const COLUMN_FAMILY_PROOFS: &str = "proof_metadata";

/// Writes a given state to the persistent log
pub(crate) fn write_state<D: SharedData, PS: PersistableStatefulOrderProtocol>(
    db: &KVDB, (view_seq, state, dec_log): InstallState<D, PS>,
) -> Result<()> {
    write_latest_view_seq_no(db, view_seq)?;

    write_checkpoint(db, state)?;

    write_dec_log(db, dec_log)
}

fn write_latest_view_seq_no(db: &KVDB, view_seq_no: SeqNo) -> Result<()> {
    let mut f_seq_no = serialize::make_seq(view_seq_no)?;

    db.set(COLUMN_FAMILY_OTHER, LATEST_VIEW_SEQ, &f_seq_no[..])
}

fn write_latest_seq_no(db: &KVDB, seq_no: SeqNo) -> Result<()> {
    let mut f_seq_no = serialize::make_seq(seq_no)?;

    if !db.exists(COLUMN_FAMILY_OTHER, FIRST_SEQ)? {
        db.set(COLUMN_FAMILY_OTHER, FIRST_SEQ, &f_seq_no[..])?;
    }

    db.set(COLUMN_FAMILY_OTHER, LATEST_SEQ, &f_seq_no[..])
}

fn write_checkpoint<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, checkpoint: Arc<ReadOnly<Checkpoint<D::State>>>) -> Result<()> {
    let mut state = Vec::new();

    D::serialize_state(&mut state, checkpoint.state())?;

    db.set(COLUMN_FAMILY_OTHER, LATEST_STATE, state.as_slice())?;

    let seq_no = serialize::make_seq(checkpoint.sequence_number())?;

    //Only remove the previous operations after persisting the checkpoint,
    //To assert no information can be lost
    let start = db.get(COLUMN_FAMILY_OTHER, FIRST_SEQ)?.unwrap();

    let start = serialize::read_seq(&start[..])?;

    //Update the first seq number, officially making all of the previous messages useless
    //And ready to be deleted
    db.set(COLUMN_FAMILY_OTHER, FIRST_SEQ, seq_no.as_slice())?;

    delete_proofs_between(db, start, checkpoint.sequence_number())?;

    Ok(())
}

fn write_dec_log<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, dec_log: &PSDecLog<PS>) -> Result<()> {
    write_latest_seq_no(db, dec_log.sequence_number())?;

    for proof_ref in PS::decompose_dec_log(dec_log) {
        write_proof(db, proof_ref)?;
    }

    Ok(())
}

fn write_proof<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, proof: &PSProof<PS>) -> Result<()> {
    let (proof_metadata, messages) = PS::decompose_proof(proof);

    write_proof_metadata(db, proof_metadata)?;

    for message in messages {
        write_message(db, message)?;
    }

    Ok(())
}

fn write_message<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, message: &StoredMessage<PSMessage<PS>>) -> Result<()> {
    let mut buf = Vec::with_capacity(Header::LENGTH + message.header().payload_length());

    message.header().serialize_into(&mut buf[..Header::LENGTH]).unwrap();

    serialize::serialize_message::<[u8], PS>(&mut buf[Header::LENGTH..], message.message())?;

    let msg_seq = message.message().sequence_number();

    let key = serialize::make_message_key(msg_seq, Some(message.header().from()))?;

    let column_family = PS::get_type_for_message(message.message())?;

    db.set(column_family, key, buf)
}

fn write_proof_metadata<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, proof_metadata: &PS::ProofMetadata) -> Result<()> {
    let seq_no = serialize::make_seq(proof_metadata.sequence_number())?;

    let mut proof_vec = Vec::new();

    let _ = serialize::serialize_proof_metadata::<Vec<u8>, PS>(&mut proof_vec, proof_metadata)?;

    db.set(COLUMN_FAMILY_PROOFS, seq_no, &proof_vec[..])
}

fn delete_proofs_between<PS: PersistableStatefulOrderProtocol>(db: &KVDB, start: SeqNo, end: SeqNo) -> Result<()> {
    let start = serialize::make_seq(start)?;
    let end = serialize::make_seq(end)?;

    for column_family in PS::message_types() {
        //Erase all of the messages
        db.erase_range(column_family, start.as_slice(), end.as_slice())?;
    }

    // Erase all of the proof metadata
    db.erase_range(COLUMN_FAMILY_PROOFS, start.as_slice(), end.as_slice())?;

    Ok(())
}