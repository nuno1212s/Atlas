use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Context;
use log::error;

use atlas_common::channel::oneshot::OneShotTx;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::persistentdb::KVDB;
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::{Header, StoredMessage};
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::{OrderProtocolLogHelper, PProof};
use atlas_core::ordering_protocol::networking::serialize::{
    OrderingProtocolMessage, PermissionedOrderingProtocolMessage,
};
use atlas_core::ordering_protocol::{
    DecisionAD, DecisionMetadata, ProtocolMessage, ShareableMessage, View,
};
use atlas_core::persistent_log::PersistableStateTransferProtocol;
use atlas_logging_core::decision_log::serialize::DecisionLogMessage;
use atlas_logging_core::decision_log::{DecLog, DecLogMetadata, DecisionLogPersistenceHelper};
use atlas_smr_application::state::divisible_state::DivisibleState;

use crate::stateful_logs::divisible_state::DivisibleStateMessage;
use crate::{serialize, CallbackType, ChannelMsg, InstallState, PWMessage, ResponseMessage};

pub(super) mod divisible_state_worker;
pub(super) mod monolithic_worker;

///Latest checkpoint made by the execution
#[allow(dead_code)]
const LATEST_STATE: &str = "latest_state";

///First sequence number (committed) since the last checkpoint
const FIRST_SEQ: &str = "first_seq";
///Last sequence number (committed) since the last checkpoint
const LATEST_SEQ: &str = "latest_seq";
///Latest known view sequence number
#[allow(dead_code)]
const LATEST_VIEW_SEQ: &str = "latest_view_seq";
/// Metadata of the decision log
const DECISION_LOG_METADATA: &str = "dec_log_metadata";

/// The default column family for the persistent logging
pub(super) const COLUMN_FAMILY_OTHER: &str = "other";
pub(super) const COLUMN_FAMILY_PROOFS: &str = "proof_metadata";
#[allow(dead_code)]
pub(super) const COLUMN_FAMILY_ADDITIONAL_DATA: &str = "additional_data";
pub(super) const COLUMN_FAMILY_STATE: &str = "state";

/// A handle for all of the persistent workers.
/// Handles task distribution and load balancing across the
/// workers
pub struct PersistentLogWorkerHandle<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
> {
    round_robin_counter: AtomicUsize,
    tx: Vec<PersistentLogWriteStub<RQ, OPM, POPT, LS>>,
}

///A stub that is only useful for writing to the persistent log
#[derive(Clone)]
pub struct PersistentLogWriteStub<RQ, OPM, POPT, LS>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
{
    pub(crate) tx: ChannelSyncTx<ChannelMsg<RQ, OPM, POPT, LS>>,
}

/// A writing stub for divisible state objects
#[allow(dead_code)]
#[derive(Clone)]
pub struct PersistentDivisibleStateStub<S: DivisibleState> {
    pub(crate) tx: ChannelSyncTx<DivisibleStateMessage<S>>,
}

impl<RQ, OPM, POPT, LS> PersistentLogWorkerHandle<RQ, OPM, POPT, LS>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ> + 'static,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
{
    pub fn new(tx: Vec<PersistentLogWriteStub<RQ, OPM, POPT, LS>>) -> Self {
        Self {
            round_robin_counter: AtomicUsize::new(0),
            tx,
        }
    }
}

///A worker for the persistent logging
pub struct PersistentLogWorker<RQ, OPM, POPT, LS, PSP, POPH, DLPH>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ> + 'static,
    POPT: PersistentOrderProtocolTypes<RQ, OPM> + 'static,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
    PSP: PersistableStateTransferProtocol + 'static,
    POPH: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
    DLPH: DecisionLogPersistenceHelper<RQ, OPM, POPT, LS> + 'static,
{
    request_rx: ChannelSyncRx<ChannelMsg<RQ, OPM, POPT, LS>>,

    response_txs: Vec<ChannelSyncTx<ResponseMessage>>,

    db: KVDB,

    phantom: PhantomData<(RQ, PSP, POPH, DLPH)>,
}

impl<RQ, OPM, POPT, LS> PersistentLogWorkerHandle<RQ, OPM, POPT, LS>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
{
    /// Employ a simple round robin load distribution
    fn next_worker(&self) -> &PersistentLogWriteStub<RQ, OPM, POPT, LS> {
        let counter = self.round_robin_counter.fetch_add(1, Ordering::Relaxed);

        self.tx.get(counter % self.tx.len()).unwrap()
    }

    /// TODO: Finish this implementation
    #[allow(dead_code)]
    pub(super) fn register_callback_receiver(
        &self,
        receiver: ChannelSyncTx<ResponseMessage>,
    ) -> Result<()> {
        for write_stub in &self.tx {
            write_stub
                .tx
                .send((PWMessage::RegisterCallbackReceiver(receiver.clone()), None))?;
        }

        Ok(())
    }

    pub fn queue_decision_log_checkpoint(
        &self,
        seq_no: SeqNo,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::DecisionLogCheckpointed(seq_no), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(super) fn queue_invalidate(
        &self,
        seq_no: SeqNo,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::Invalidate(seq_no), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(crate) fn queue_committed(
        &self,
        seq_no: SeqNo,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::Committed(seq_no), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(super) fn queue_proof_metadata(
        &self,
        metadata: DecisionMetadata<RQ, OPM>,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::ProofMetadata(metadata), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(super) fn queue_read_proof(
        &self,
        seq: SeqNo,
        proof_return: OneShotTx<Option<PProof<RQ, OPM, POPT>>>,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::ReadProof(seq, proof_return), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(super) fn queue_read_dec_log(
        &self,
        decision_log: OneShotTx<Option<DecLog<RQ, OPM, POPT, LS>>>,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::ReadDecisionLog(decision_log), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(super) fn queue_message(
        &self,
        message: ShareableMessage<ProtocolMessage<RQ, OPM>>,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::Message(message), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(super) fn queue_decision_log_metadata(
        &self,
        decision_log_meta: DecLogMetadata<RQ, OPM, POPT, LS>,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::DecisionLogMetadata(decision_log_meta), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(super) fn queue_install_state(
        &self,
        install_state: InstallState<RQ, OPM, POPT, LS>,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::InstallState(install_state), callback))
            .context("Failed placing message into persistent log worker")
    }

    pub(super) fn queue_proof(
        &self,
        proof: PProof<RQ, OPM, POPT>,
        callback: Option<CallbackType>,
    ) -> Result<()> {
        self.next_worker()
            .tx
            .send((PWMessage::Proof(proof), callback))
            .context("Failed placing message into persistent log worker")
    }
}

impl<RQ, OPM, POPT, LS, PS, DLPS, PSP> PersistentLogWorker<RQ, OPM, POPT, LS, PSP, PS, DLPS>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ> + 'static,
    POPT: PersistentOrderProtocolTypes<RQ, OPM> + 'static,
    LS: DecisionLogMessage<RQ, OPM, POPT> + 'static,
    PSP: PersistableStateTransferProtocol + 'static,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
    DLPS: DecisionLogPersistenceHelper<RQ, OPM, POPT, LS> + 'static,
{
    pub fn new(
        request_rx: ChannelSyncRx<ChannelMsg<RQ, OPM, POPT, LS>>,
        response_txs: Vec<ChannelSyncTx<ResponseMessage>>,
        db: KVDB,
    ) -> Self {
        Self {
            request_rx,
            response_txs,
            db,
            phantom: Default::default(),
        }
    }

    fn work_iteration(&mut self) -> Result<()> {
        let (request, callback) = self.request_rx.recv()?;

        let response = self.exec_req(request)?;

        if let Some(_) = callback {
            //If we have a callback to call with the response, then call it
            //TODO: Implement callback handling
            // (callback)(response);
        } else {
            //If not, then deliver it down the response_txs
            for ele in &self.response_txs {
                ele.send(response.clone())?;
            }
        }

        Ok(())
    }

    /// Work loop of this worker
    pub(super) fn work(mut self) {
        loop {
            if let Err(err) = self.work_iteration() {
                error!("Failed to execute persistent log request because {:?}", err);

                break;
            }
        }
    }

    fn exec_req(&mut self, message: PWMessage<RQ, OPM, POPT, LS>) -> Result<ResponseMessage> {
        Ok(match message {
            PWMessage::DecisionLogCheckpointed(seq) => {
                ResponseMessage::DecisionLogCheckpointPersisted(seq)
            }
            PWMessage::Committed(seq) => {
                write_latest_seq_no(&self.db, seq)?;

                ResponseMessage::CommittedPersisted(seq)
            }
            PWMessage::Message(msg) => {
                write_message::<RQ, OPM, POPT, PS>(&self.db, &msg)?;

                let seq = msg.message().sequence_number();

                ResponseMessage::WroteMessage(seq, msg.header().digest().clone())
            }
            PWMessage::Invalidate(seq) => {
                invalidate_seq::<RQ, OPM, POPT, PS>(&self.db, seq)?;

                ResponseMessage::InvalidationPersisted(seq)
            }
            PWMessage::InstallState(state) => {
                let seq_no = state.0.sequence_number();

                write_state::<RQ, OPM, POPT, LS, PS, DLPS>(&self.db, state)?;

                ResponseMessage::InstalledState(seq_no)
            }
            PWMessage::Proof(proof) => {
                let seq_no = proof.sequence_number();

                write_proof::<RQ, OPM, POPT, PS>(&self.db, &proof)?;

                ResponseMessage::Proof(seq_no)
            }
            PWMessage::RegisterCallbackReceiver(receiver) => {
                self.response_txs.push(receiver);

                ResponseMessage::RegisteredCallback
            }
            PWMessage::ProofMetadata(metadata) => {
                let seq = metadata.sequence_number();

                write_proof_metadata::<RQ, OPM, POPT, PS>(&self.db, &metadata)?;

                ResponseMessage::WroteMetadata(seq)
            }
            PWMessage::DecisionLogMetadata(metadata) => {
                write_decision_log_metadata::<RQ, OPM, POPT, LS>(&self.db, metadata)?;

                ResponseMessage::WroteDecLogMetadata
            }
            PWMessage::ReadProof(seq, shot) => {
                shot.send(read_proof::<RQ, OPM, POPT, PS>(&self.db, seq)?)
                    .expect("failed to response to proof");

                ResponseMessage::ReadProof
            }
            PWMessage::ReadDecisionLog(shot) => {
                shot.send(
                    read_latest_state::<RQ, OPM, POPT, LS, PS, DLPS>(&self.db)?
                        .map(|state| state.0),
                )
                .expect("failed to respond to decision log");

                ResponseMessage::ReadDecisionLog
            }
        })
    }
}

/// Read the latest state from the persistent log
pub(super) fn read_latest_state<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT>,
    PLS: DecisionLogPersistenceHelper<RQ, OPM, POPT, LS>,
>(
    db: &KVDB,
) -> Result<Option<InstallState<RQ, OPM, POPT, LS>>> {
    let dec_log = read_decision_log::<RQ, OPM, POPT, LS, PS, PLS>(db)?;

    if let None = &dec_log {
        return Ok(None);
    }

    Ok(Some((dec_log.unwrap(),)))
}

fn read_proof_metadata<RQ: SerMsg, OPM: OrderingProtocolMessage<RQ>>(
    db: &KVDB,
    seq: SeqNo,
) -> Result<Option<DecisionMetadata<RQ, OPM>>> {
    let first_seq = serialize::make_seq(seq)?;

    let proof_metadata = db.get(COLUMN_FAMILY_PROOFS, &first_seq[..])?;

    Ok(if let Some(metadata) = proof_metadata {
        Some(serialize::deserialize_proof_metadata::<&[u8], RQ, OPM>(
            &mut metadata.as_ref(),
        )?)
    } else {
        None
    })
}

fn read_proof_additional_data<RQ: SerMsg, OPM: OrderingProtocolMessage<RQ>>(
    _db: &KVDB,
    _seq: SeqNo,
) -> Result<Vec<DecisionAD<RQ, OPM>>> {
    todo!()
}

pub(super) fn read_proof<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT>,
>(
    db: &KVDB,
    seq: SeqNo,
) -> Result<Option<PProof<RQ, OPM, POPT>>> {
    let metadata = read_proof_metadata::<RQ, OPM>(db, seq)?;

    if let None = &metadata {
        return Ok(None);
    }

    let additional_data = read_proof_additional_data::<RQ, OPM>(db, seq)?;

    let metadata = metadata.unwrap();

    let messages = read_messages_for_seq::<RQ, OPM, POPT, PS>(db, seq)?;

    Ok(Some(PS::init_proof_from(
        metadata,
        additional_data,
        messages,
    )?))
}

fn read_decision_log<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT>,
    PLS: DecisionLogPersistenceHelper<RQ, OPM, POPT, LS>,
>(
    db: &KVDB,
) -> Result<Option<DecLog<RQ, OPM, POPT, LS>>> {
    let first_seq = db.get(COLUMN_FAMILY_OTHER, FIRST_SEQ)?;
    let last_seq = db.get(COLUMN_FAMILY_OTHER, LATEST_SEQ)?;

    let decision_log_metadata = db.get(COLUMN_FAMILY_OTHER, DECISION_LOG_METADATA)?;

    let decision_log_metadata = if let Some(dec_log) = decision_log_metadata {
        serialize::deserialize_decision_log_metadata::<&[u8], RQ, OPM, POPT, LS>(
            &mut dec_log.as_ref(),
        )?
    } else {
        return Ok(None);
    };

    let start_seq = if let Some(first_seq) = first_seq {
        serialize::read_seq(first_seq.as_ref())?
    } else {
        return Ok(None);
    };

    let end_seq = if let Some(end_seq) = last_seq {
        serialize::read_seq(end_seq.as_ref())?
    } else {
        return Ok(None);
    };

    let start_point = serialize::make_seq(start_seq)?;
    let end_point = serialize::make_seq(end_seq.next())?;

    let mut proofs = Vec::new();

    for result in db.iter_range(
        COLUMN_FAMILY_PROOFS,
        Some(start_point.as_slice()),
        Some(end_point.as_slice()),
    )? {
        let (key, value) = result?;

        let seq = serialize::read_seq(key.as_ref())?;

        let proof_metadata =
            serialize::deserialize_proof_metadata::<&[u8], RQ, OPM>(&mut value.as_ref())?;

        let ad = Vec::new();

        let messages = read_messages_for_seq::<RQ, OPM, POPT, PS>(db, seq)?;

        let proof = PS::init_proof_from(proof_metadata, ad, messages)?;

        proofs.push(proof);
    }

    Ok(Some(PLS::init_decision_log(decision_log_metadata, proofs)?))
}

fn read_messages_for_seq<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT>,
>(
    db: &KVDB,
    seq: SeqNo,
) -> Result<Vec<StoredMessage<ProtocolMessage<RQ, OPM>>>> {
    let start_seq = serialize::make_message_key(seq, None)?;
    let end_seq = serialize::make_message_key(seq.next(), None)?;

    let mut messages = Vec::new();

    for column_family in PS::message_types() {
        for result in db.iter_range(
            column_family,
            Some(start_seq.as_slice()),
            Some(end_seq.as_slice()),
        )? {
            let (_key, value) = result?;

            let header = Header::deserialize_from(&value.as_ref()[..Header::LENGTH])?;

            let message = serialize::deserialize_message::<&[u8], RQ, OPM>(
                &mut &value.as_ref()[Header::LENGTH..],
            )?;

            messages.push(StoredMessage::new(header, message));
        }
    }

    Ok(messages)
}

#[allow(dead_code)]
fn read_latest_view_seq<POP: PermissionedOrderingProtocolMessage>(
    db: &KVDB,
) -> Result<Option<View<POP>>> {
    let result = db.get(COLUMN_FAMILY_OTHER, LATEST_VIEW_SEQ)?;

    Ok(if let Some(result) = result {
        Some(serialize::deserialize_view::<&[u8], POP>(
            &mut result.as_ref(),
        )?)
    } else {
        None
    })
}

/// Writes a given state to the persistent log
pub(super) fn write_state<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT>,
    PLS: DecisionLogPersistenceHelper<RQ, OPM, POPT, LS>,
>(
    db: &KVDB,
    dec_log: InstallState<RQ, OPM, POPT, LS>,
) -> Result<()> {
    write_dec_log::<RQ, OPM, POPT, PS, LS, PLS>(db, dec_log.0)
}

#[allow(dead_code)]
pub(super) fn write_latest_view<POP: PermissionedOrderingProtocolMessage>(
    db: &KVDB,
    view_seq_no: &View<POP>,
) -> Result<()> {
    let mut res = Vec::new();

    serialize::serialize_view::<Vec<u8>, POP>(&mut res, view_seq_no)?;

    db.set(COLUMN_FAMILY_OTHER, LATEST_VIEW_SEQ, &res[..])
}

pub(super) fn write_latest_seq_no(db: &KVDB, seq_no: SeqNo) -> Result<()> {
    let f_seq_no = serialize::make_seq(seq_no)?;

    if !db.exists(COLUMN_FAMILY_OTHER, FIRST_SEQ)? {
        db.set(COLUMN_FAMILY_OTHER, FIRST_SEQ, &f_seq_no[..])?;
    }

    db.set(COLUMN_FAMILY_OTHER, LATEST_SEQ, &f_seq_no[..])
}

pub(super) fn write_dec_log<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
    PLS: DecisionLogPersistenceHelper<RQ, OPM, POPT, LS>,
>(
    db: &KVDB,
    dec_log: DecLog<RQ, OPM, POPT, LS>,
) -> Result<()> {
    write_latest_seq_no(db, dec_log.sequence_number())?;

    let (metadata, proofs) = PLS::decompose_decision_log(dec_log);

    write_decision_log_metadata::<RQ, OPM, POPT, LS>(db, metadata)?;

    for proof_ref in proofs {
        write_proof::<RQ, OPM, POPT, PS>(db, &proof_ref)?;
    }

    Ok(())
}

pub(super) fn write_proof<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
>(
    db: &KVDB,
    proof: &PProof<RQ, OPM, POPT>,
) -> Result<()> {
    let (proof_metadata, additional_data, messages) = PS::decompose_proof(proof);

    write_proof_metadata::<RQ, OPM, POPT, PS>(db, proof_metadata)?;

    additional_data
        .into_iter()
        .try_for_each(|ad| write_additional_data::<RQ, OPM, POPT, PS>(db, ad))?;

    messages
        .into_iter()
        .try_for_each(|msg| write_message::<RQ, OPM, POPT, PS>(db, msg))?;

    Ok(())
}

pub(super) fn write_additional_data<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
>(
    _db: &KVDB,
    _additional_data: &DecisionAD<RQ, OPM>,
) -> Result<()> {
    todo!()
}

pub(super) fn write_message<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
>(
    db: &KVDB,
    message: &StoredMessage<ProtocolMessage<RQ, OPM>>,
) -> Result<()> {
    let mut buf = Vec::with_capacity(Header::LENGTH + message.header().payload_length());

    message
        .header()
        .serialize_into(&mut buf[..Header::LENGTH])?;

    serialize::serialize_message::<&mut [u8], RQ, OPM>(
        &mut &mut buf[Header::LENGTH..],
        message.message(),
    )?;

    let msg_seq = message.message().sequence_number();

    let key = serialize::make_message_key(msg_seq, Some(message.header().from()))?;

    let column_family = PS::get_type_for_message(message.message())?;

    db.set(column_family, key, buf)
}

pub(super) fn write_decision_log_metadata<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
>(
    db: &KVDB,
    decision: DecLogMetadata<RQ, OPM, POPT, LS>,
) -> Result<()> {
    let mut decision_log_meta = Vec::new();

    let _ = serialize::serialize_decision_log_metadata::<Vec<u8>, RQ, OPM, POPT, LS>(
        &mut decision_log_meta,
        &decision,
    );

    db.set(
        COLUMN_FAMILY_OTHER,
        DECISION_LOG_METADATA,
        &decision_log_meta[..],
    )
}

pub(super) fn write_proof_metadata<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
>(
    db: &KVDB,
    proof_metadata: &DecisionMetadata<RQ, OPM>,
) -> Result<()> {
    let seq_no = serialize::make_seq(proof_metadata.sequence_number())?;

    let mut proof_vec = Vec::new();

    let _ =
        serialize::serialize_proof_metadata::<Vec<u8>, RQ, OPM>(&mut proof_vec, proof_metadata)?;

    db.set(COLUMN_FAMILY_PROOFS, seq_no, &proof_vec[..])
}

#[allow(dead_code)]
//TODO: Use this function to delete proofs between two sequence numbers
fn delete_proofs_between<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
>(
    db: &KVDB,
    start: SeqNo,
    end: SeqNo,
) -> Result<()> {
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

pub(super) fn invalidate_seq<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
>(
    db: &KVDB,
    seq: SeqNo,
) -> Result<()> {
    delete_all_msgs_for_seq::<RQ, OPM, POPT, PS>(db, seq)?;
    delete_all_proof_metadata_for_seq(db, seq)?;

    Ok(())
}

///Delete all msgs relating to a given sequence number
fn delete_all_msgs_for_seq<
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    PS: OrderProtocolLogHelper<RQ, OPM, POPT> + 'static,
>(
    db: &KVDB,
    msg_seq: SeqNo,
) -> Result<()> {
    let start_key = serialize::make_message_key(msg_seq, None)?;
    let end_key = serialize::make_message_key(msg_seq.next(), None)?;

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

impl<RQ, OPM, POPT, LS> Deref for PersistentLogWriteStub<RQ, OPM, POPT, LS>
where
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OPM>,
    LS: DecisionLogMessage<RQ, OPM, POPT>,
{
    type Target = ChannelSyncTx<ChannelMsg<RQ, OPM, POPT, LS>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
