use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use log::error;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx, SendError};
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::persistentdb::KVDB;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_communication::message::{Header, StoredMessage};
use atlas_core::state_transfer::Checkpoint;
use atlas_execution::serialize::SharedData;
use crate::{CallbackType, ChannelMsg, InstallState, PWMessage, ResponseMessage, serialize};
use atlas_core::persistent_log::{PersistableOrderProtocol, PSDecLog, PSMessage, PSProof, PSView};


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
pub struct PersistentLogWorkerHandle<D: SharedData, PS: PersistableOrderProtocol> {
    round_robin_counter: AtomicUsize,
    tx: Vec<PersistentLogWriteStub<D, PS>>,
}

///A stub that is only useful for writing to the persistent log
#[derive(Clone)]
pub(super) struct PersistentLogWriteStub<D: SharedData, PS: PersistableOrderProtocol> {
    pub(crate) tx: ChannelSyncTx<ChannelMsg<D, PS>>,
}

impl<D, PS> PersistentLogWorkerHandle<D, PS> where D: SharedData, PS: PersistableOrderProtocol {
    pub fn new(tx: Vec<PersistentLogWriteStub<D, PS>>) -> Self {
        Self { round_robin_counter: AtomicUsize::new(0), tx }
    }
}


///A worker for the persistent logging
pub struct PersistentLogWorker<D: SharedData, PS: PersistableOrderProtocol> {
    request_rx: ChannelSyncRx<ChannelMsg<D, PS>>,

    response_txs: Vec<ChannelSyncTx<ResponseMessage>>,

    db: KVDB,
}


impl<D: SharedData, PS: PersistableOrderProtocol> PersistentLogWorkerHandle<D, PS> {
    /// Employ a simple round robin load distribution
    fn next_worker(&self) -> &PersistentLogWriteStub<D, PS> {
        let counter = self.round_robin_counter.fetch_add(1, Ordering::Relaxed);

        self.tx.get(counter % self.tx.len()).unwrap()
    }

    fn translate_error<V, T>(result: std::result::Result<V, SendError<T>>) -> Result<V> {
        match result {
            Ok(v) => {
                Ok(v)
            }
            Err(err) => {
                Err(Error::simple_with_msg(ErrorKind::MsgLogPersistent, format!("{:?}", err).as_str()))
            }
        }
    }

    pub(super) fn register_callback_receiver(&self, receiver: ChannelSyncTx<ResponseMessage>) -> Result<()> {
        for write_stub in &self.tx {
            Self::translate_error(write_stub.send((PWMessage::RegisterCallbackReceiver(receiver.clone()), None)))?;
        }

        Ok(())
    }

    pub(super) fn queue_invalidate(&self, seq_no: SeqNo, callback: Option<CallbackType>) -> Result<()> {
        Self::translate_error(self.next_worker().send((PWMessage::Invalidate(seq_no), callback)))
    }

    pub(crate) fn queue_committed(&self, seq_no: SeqNo, callback: Option<CallbackType>) -> Result<()> {
        Self::translate_error(self.next_worker().send((PWMessage::Committed(seq_no), callback)))
    }

    pub(super) fn queue_proof_metadata(&self, metadata: PS::ProofMetadata, callback: Option<CallbackType>) -> Result<()> {
        Self::translate_error(self.next_worker().send((PWMessage::ProofMetadata(metadata), callback)))
    }

    pub(super) fn queue_view_number(&self, view: PSView<PS>, callback: Option<CallbackType>) -> Result<()> {
        Self::translate_error(self.next_worker().send((PWMessage::View(view), callback)))
    }

    pub(super) fn queue_message(&self, message: Arc<ReadOnly<StoredMessage<PSMessage<PS>>>>,
                                callback: Option<CallbackType>) -> Result<()> {
        Self::translate_error(self.next_worker().send((PWMessage::Message(message), callback)))
    }

    pub(super) fn queue_state(&self, state: Arc<ReadOnly<Checkpoint<D::State>>>, callback: Option<CallbackType>) -> Result<()> {
        Self::translate_error(self.next_worker().send((PWMessage::Checkpoint(state), callback)))
    }

    pub(super) fn queue_install_state(&self, install_state: InstallState<PS>, callback: Option<CallbackType>) -> Result<()> {
        Self::translate_error(self.next_worker().send((PWMessage::InstallState(install_state), callback)))
    }

    pub(super) fn queue_proof(&self, proof: PSProof<PS>, callback: Option<CallbackType>) -> Result<()> {
        Self::translate_error(self.next_worker().send((PWMessage::Proof(proof), callback)))
    }
}


impl<D: SharedData, PS: PersistableOrderProtocol> PersistentLogWorker<D, PS> {
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
                write_message::<PS>(&self.db, &msg)?;

                let seq = msg.message().sequence_number();

                ResponseMessage::WroteMessage(seq, msg.header().digest().clone())
            }
            PWMessage::Checkpoint(checkpoint) => {
                write_checkpoint::<D, PS>(&self.db, checkpoint)?;

                ResponseMessage::Checkpointed(checkpoint.sequence_number())
            }
            PWMessage::Invalidate(seq) => {
                invalidate_seq::<PS>(&self.db, seq)?;

                ResponseMessage::InvalidationPersisted(seq)
            }
            PWMessage::InstallState(state) => {
                let seq_no = state.1.sequence_number();

                write_state::<D, PS>(&self.db, state)?;

                ResponseMessage::InstalledState(seq_no)
            }
            PWMessage::Proof(proof) => {
                let seq_no = proof.sequence_number();

                write_proof::<PS>(&self.db, &proof)?;

                ResponseMessage::Proof(seq_no)
            }
            PWMessage::RegisterCallbackReceiver(receiver) => {
                self.response_txs.push(receiver);

                ResponseMessage::RegisteredCallback
            }
            PWMessage::ProofMetadata(metadata) => {
                let seq = metadata.sequence_number();

                write_proof_metadata::<PS>(&self.db, &metadata)?;

                ResponseMessage::WroteMetadata(seq)
            }
        })
    }
}

/// Read the latest state from the persistent log
pub(super) fn read_latest_state<PS: PersistableOrderProtocol>(db: &KVDB) -> Result<Option<InstallState<PS>>> {
    let view_seq = read_latest_view_seq(db)?;

    if let None = view_seq {
        return Ok(None);
    }

    let dec_log = read_decision_log::<PS>(db)?;

    if let None = dec_log {
        return Ok(None);
    }

    Ok(Some((view_seq.unwrap(), dec_log.unwrap())))
}

fn read_decision_log<PS: PersistableOrderProtocol>(db: &KVDB) -> Result<Option<PSDecLog<PS>>> {
    let first_seq = db.get(COLUMN_FAMILY_OTHER, FIRST_SEQ)?;
    let last_seq = db.get(COLUMN_FAMILY_OTHER, LATEST_SEQ)?;

    let start_seq = if let Some(first_seq) = first_seq {
        serialize::read_seq(first_seq.as_slice())?
    } else {
        return Ok(None);
    };

    let end_seq = if let Some(end_seq) = last_seq {
        serialize::read_seq(end_seq.as_slice())?
    } else {
        return Ok(None);
    };

    let start_point = serialize::make_seq(start_seq)?;
    let end_point = serialize::make_seq(end_seq.next())?;

    let mut proofs = Vec::new();

    for result in db.iter_range(COLUMN_FAMILY_PROOFS, Some(start_point.as_slice()), Some(end_point.as_slice()))? {
        let (key, value) = result?;

        let seq = serialize::read_seq(&*key)?;

        let proof_metadata = serialize::deserialize_proof_metadata::<&[u8], PS>(&*value)?;

        let messages = read_messages_for_seq::<PS>(db, seq)?;

        let proof = PS::init_proof_from(proof_metadata, messages);

        proofs.push(proof);
    }

    Ok(Some(PS::init_dec_log(proofs)))
}

fn read_messages_for_seq<PS: PersistableOrderProtocol>(db: &KVDB, seq: SeqNo) -> Result<Vec<PSMessage<PS>>> {
    let start_seq = serialize::make_message_key(seq, None)?;
    let end_seq = serialize::make_message_key(seq.next(), None)?;

    let mut messages = Vec::new();

    for column_family in PS::message_types() {
        for result in db.iter_range(column_family, Some(start_seq.as_slice()), Some(end_seq.as_slice()))? {
            let (key, value) = result?;

            let message = serialize::deserialize_message::<&[u8], PS>(&*value).unwrap();

            messages.push(message);
        }
    }

    Ok(messages)
}

fn read_latest_view_seq(db: &KVDB) -> Result<Option<SeqNo>> {
    let result = db.get(COLUMN_FAMILY_OTHER, LATEST_VIEW_SEQ)?;

    let option = result.map(|result| serialize::read_seq(result.as_slice()));

    if let Some(Ok(seq)) = option {
        return Ok(Some(seq));
    } else {
        return Ok(None);
    }
}


/// Writes a given state to the persistent log
pub(super) fn write_state<D: SharedData, PS: PersistableOrderProtocol>(
    db: &KVDB, (view_seq, dec_log): InstallState<PS>,
) -> Result<()> {
    write_latest_view_seq_no(db, view_seq)?;

    write_dec_log::<PS>(db, &dec_log)
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

pub(super) fn write_checkpoint<D: SharedData, PS: PersistableOrderProtocol>(db: &KVDB, checkpoint: Arc<ReadOnly<Checkpoint<D::State>>>) -> Result<()> {
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

    delete_proofs_between::<PS>(db, start, checkpoint.sequence_number())?;

    Ok(())
}

pub(super) fn write_dec_log<PS: PersistableOrderProtocol>(db: &KVDB, dec_log: &PSDecLog<PS>) -> Result<()> {
    write_latest_seq_no(db, dec_log.sequence_number())?;

    for proof_ref in PS::decompose_dec_log(dec_log) {
        write_proof::<PS>(db, proof_ref)?;
    }

    Ok(())
}

pub(super) fn write_proof< PS: PersistableOrderProtocol>(db: &KVDB, proof: &PSProof<PS>) -> Result<()> {
    let (proof_metadata, messages) = PS::decompose_proof(proof);

    write_proof_metadata::< PS>(db, proof_metadata)?;

    for message in messages {
        write_message::<PS>(db, message)?;
    }

    Ok(())
}

pub(super) fn write_message<PS: PersistableOrderProtocol>(db: &KVDB, message: &StoredMessage<PSMessage<PS>>) -> Result<()> {
    let mut buf = Vec::with_capacity(Header::LENGTH + message.header().payload_length());

    message.header().serialize_into(&mut buf[..Header::LENGTH]).unwrap();

    serialize::serialize_message::<&mut [u8], PS>(&mut &mut buf[Header::LENGTH..], message.message())?;

    let msg_seq = message.message().sequence_number();

    let key = serialize::make_message_key(msg_seq, Some(message.header().from()))?;

    let column_family = PS::get_type_for_message(message.message())?;

    db.set(column_family, key, buf)
}

pub(super) fn write_proof_metadata<PS: PersistableOrderProtocol>(db: &KVDB, proof_metadata: &PS::ProofMetadata) -> Result<()> {
    let seq_no = serialize::make_seq(proof_metadata.sequence_number())?;

    let mut proof_vec = Vec::new();

    let _ = serialize::serialize_proof_metadata::<Vec<u8>, PS>(&mut proof_vec, proof_metadata)?;

    db.set(COLUMN_FAMILY_PROOFS, seq_no, &proof_vec[..])
}

fn delete_proofs_between<PS: PersistableOrderProtocol>(db: &KVDB, start: SeqNo, end: SeqNo) -> Result<()> {
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

pub(super) fn invalidate_seq<PS>(db: &KVDB, seq: SeqNo) -> Result<()> where PS: PersistableOrderProtocol {
    delete_all_msgs_for_seq::<PS>(db, seq)?;
    delete_all_proof_metadata_for_seq(db, seq)?;

    Ok(())
}

///Delete all msgs relating to a given sequence number
fn delete_all_msgs_for_seq<PS: PersistableOrderProtocol>(db: &KVDB, msg_seq: SeqNo) -> Result<()> {
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


impl<D: SharedData, PS: PersistableOrderProtocol> Deref for PersistentLogWriteStub<D, PS> {
    type Target = ChannelSyncTx<ChannelMsg<D, PS>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
