use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::persistentdb::KVDB;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_communication::message::{Header, StoredMessage};
use atlas_core::state_transfer::Checkpoint;
use atlas_execution::serialize::SharedData;
use crate::{ChannelMsg, InstallState, PWMessage, ResponseMessage, serialize};
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
pub(super) const COLUMN_FAMILY_OTHER: &str = "other";
pub(super) const COLUMN_FAMILY_PROOFS: &str = "proof_metadata";


/// A handle for all of the persistent workers.
/// Handles task distribution and load balancing across the
/// workers
pub struct PersistentLogWorkerHandle<D: SharedData, PS: PersistableStatefulOrderProtocol> {
    round_robin_counter: AtomicUsize,
    tx: Vec<PersistentLogWriteStub<D, PS>>,
}

///A stub that is only useful for writing to the persistent log
#[derive(Clone)]
pub(super) struct PersistentLogWriteStub<D: SharedData, PS: PersistableStatefulOrderProtocol> {
    pub(crate) tx: ChannelSyncTx<ChannelMsg<D, PS>>,
}

impl<D, PS> PersistentLogWorkerHandle<D, PS> {
    pub fn new(tx: Vec<PersistentLogWriteStub<D, PS>>) -> Self {
        Self { round_robin_counter: AtomicUsize::new(0), tx }
    }
}


///A worker for the persistent logging
pub struct PersistentLogWorker<D: SharedData, PS: PersistableStatefulOrderProtocol> {
    request_rx: ChannelSyncRx<ChannelMsg<D, PS>>,

    response_txs: Vec<ChannelSyncTx<ResponseMessage>>,

    db: KVDB,
}

impl<D: SharedData, PS: PersistableStatefulOrderProtocol> PersistentLogWorker<D, PS> {

    pub fn new(request_rx: ChannelSyncRx<ChannelMsg<D, PS>>,
               response_txs: Vec<ChannelSyncTx<ResponseMessage>>,
               db: KVDB) -> Self {
        Self { request_rx, response_txs, db }
    }

    pub(super) fn work(mut self) {
        loop {
            let (request, callback) = match self.request_rx.recv() {
                Ok((request, callback)) => (request, callback),
                Err(err) => {
                    error!("{:?}", err);
                    break;
                }
            };

            let response = self.exec_req(request);

            if let Some(callback) = callback {
                //If we have a callback to call with the response, then call it
                (callback)(response);
            } else {
                //If not, then deliver it down the response_txs
                match response {
                    Ok(response) => {
                        for ele in &self.response_txs {
                            if let Err(err) = ele.send(response.clone()) {
                                error!("Failed to deliver response to log. {:?}", err);
                            }
                        }
                    }
                    Err(err) => {
                        error!("Failed to execute persistent log request because {:?}", err);
                    }
                }
            }
        }
    }

    fn exec_req(&mut self, message: PWMessage<D, PS>) -> Result<ResponseMessage> {
        Ok(match message {
            PWMessage::View(view) => {
                write_latest_view_seq_no(&self.db, view.sequence_number())?;

                ResponseMessage::ViewPersisted(view.sequence_number())
            }
            PWMessage::Committed(seq) => {
                write_latest_seq_no(&self.db, seq)?;

                ResponseMessage::CommittedPersisted(seq)
            }
            PWMessage::Message(msg) => {
                write_message::<D, PS>(&self.db, &msg)?;

                let seq = msg.message().sequence_number();

                ResponseMessage::WroteMessage(seq, msg.header().digest().clone())
            }
            PWMessage::Checkpoint(checkpoint) => {
                write_checkpoint::<D, PS>(&self.db, checkpoint)?;

                ResponseMessage::Checkpointed(checkpoint.sequence_number())
            }
            PWMessage::Invalidate(seq) => {
                invalidate_seq(&self.db, seq)?;

                ResponseMessage::InvalidationPersisted(seq)
            }
            PWMessage::InstallState(state) => {
                let seq_no = state.2.last_execution().unwrap();

                write_state::<D, PS>(&self.db, state)?;

                ResponseMessage::InstalledState(seq_no)
            }
            PWMessage::Proof(proof) => {
                let seq_no = proof.seq_no();

                write_proof::<D, PS>(&self.db, proof)?;

                ResponseMessage::Proof(seq_no)
            }
            PWMessage::RegisterCallbackReceiver(receiver) => {
                self.response_txs.push(receiver);

                ResponseMessage::RegisteredCallback
            }
            PWMessage::ProofMetadata(metadata) => {
                let seq = metadata.seq_no();

                write_proof_metadata(&self.db, metadata)?;

                ResponseMessage::WroteMetadata(seq)
            }
        })
    }
}



/// Writes a given state to the persistent log
pub(super) fn write_state<D: SharedData, PS: PersistableStatefulOrderProtocol>(
    db: &KVDB, (view_seq, state, dec_log): InstallState<D, PS>,
) -> Result<()> {
    write_latest_view_seq_no(db, view_seq)?;

    write_checkpoint(db, state)?;

    write_dec_log(db, dec_log)
}

pub(super) fn write_latest_view_seq_no(db: &KVDB, view_seq_no: SeqNo) -> Result<()> {
    let mut f_seq_no = serialize::make_seq(view_seq_no)?;

    db.set(COLUMN_FAMILY_OTHER, LATEST_VIEW_SEQ, &f_seq_no[..])
}

pub(super) fn write_latest_seq_no(db: &KVDB, seq_no: SeqNo) -> Result<()> {
    let mut f_seq_no = serialize::make_seq(seq_no)?;

    if !db.exists(COLUMN_FAMILY_OTHER, FIRST_SEQ)? {
        db.set(COLUMN_FAMILY_OTHER, FIRST_SEQ, &f_seq_no[..])?;
    }

    db.set(COLUMN_FAMILY_OTHER, LATEST_SEQ, &f_seq_no[..])
}

pub(super) fn write_checkpoint<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, checkpoint: Arc<ReadOnly<Checkpoint<D::State>>>) -> Result<()> {
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

pub(super) fn write_dec_log<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, dec_log: &PSDecLog<PS>) -> Result<()> {
    write_latest_seq_no(db, dec_log.sequence_number())?;

    for proof_ref in PS::decompose_dec_log(dec_log) {
        write_proof(db, proof_ref)?;
    }

    Ok(())
}

pub(super) fn write_proof<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, proof: &PSProof<PS>) -> Result<()> {
    let (proof_metadata, messages) = PS::decompose_proof(proof);

    write_proof_metadata(db, proof_metadata)?;

    for message in messages {
        write_message(db, message)?;
    }

    Ok(())
}

pub(super) fn write_message<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, message: &StoredMessage<PSMessage<PS>>) -> Result<()> {
    let mut buf = Vec::with_capacity(Header::LENGTH + message.header().payload_length());

    message.header().serialize_into(&mut buf[..Header::LENGTH]).unwrap();

    serialize::serialize_message::<[u8], PS>(&mut buf[Header::LENGTH..], message.message())?;

    let msg_seq = message.message().sequence_number();

    let key = serialize::make_message_key(msg_seq, Some(message.header().from()))?;

    let column_family = PS::get_type_for_message(message.message())?;

    db.set(column_family, key, buf)
}

pub(super) fn write_proof_metadata<D: SharedData, PS: PersistableStatefulOrderProtocol>(db: &KVDB, proof_metadata: &PS::ProofMetadata) -> Result<()> {
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

pub(super) fn invalidate_seq(db: &KVDB, seq: SeqNo) -> Result<()> {
    delete_all_msgs_for_seq(db, seq)?;
    delete_all_proof_metadata_for_seq(db, seq)?;

    Ok(())
}

///Delete all msgs relating to a given sequence number
fn delete_all_msgs_for_seq<PS: PersistableStatefulOrderProtocol>(db: &KVDB, msg_seq: SeqNo) -> Result<()> {
    let mut start_key =
        serialize::make_message_key(msg_seq, None)?;
    let mut end_key =
        serialize::make_message_key(msg_seq.next(), None)?;

    for column_family in PS::message_types() {
        //Erase all of the messages
        db.erase_range(column_family, start_key.as_slice(), end_key.as_slice())?;
    }

    Ok(())
}

fn delete_all_proof_metadata_for_seq(db: &KVDB, seq: SeqNo) -> Result<()> {
    let seq = serialize::make_seq(seq)?;

    db.erase(COLUMN_FAMILY_PROOFS, &seq)?;

    Ok(())
}
