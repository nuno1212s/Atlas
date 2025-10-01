use crate::serialize::{
    deserialize_state_descriptor, deserialize_state_part, serialize_state_descriptor,
    serialize_state_part, serialize_state_part_descriptor,
};
use crate::stateful_logs::divisible_state::DivisibleStateMessage;
use crate::worker::{PersistentLogWorker, COLUMN_FAMILY_STATE};
use crate::ResponseMessage;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::channel::TryRecvError;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::persistentdb::KVDB;
use atlas_common::serialization_helper::SerMsg;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::OrderProtocolLogHelper;
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_core::persistent_log::PersistableStateTransferProtocol;
use atlas_logging_core::decision_log::serialize::DecisionLogMessage;
use atlas_logging_core::decision_log::DecisionLogPersistenceHelper;
use atlas_smr_application::state::divisible_state::{DivisibleState, StatePart};
use log::error;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use anyhow::Context;
use atlas_common::quiet_unwrap;

#[derive(Clone)]
pub struct PersistentDivStateStub<S: DivisibleState> {
    pub(crate) tx: ChannelSyncTx<DivisibleStateMessage<S>>,
}

pub struct PersistentDivStateHandle<S: DivisibleState> {
    round_robin_counter: AtomicUsize,
    tx: Vec<PersistentDivStateStub<S>>,
}

#[allow(dead_code)]
impl<S> PersistentDivStateHandle<S>
where
    S: DivisibleState,
{
    pub(crate) fn new(tx: Vec<PersistentDivStateStub<S>>) -> Self {
        Self {
            round_robin_counter: AtomicUsize::new(0),
            tx,
        }
    }

    /// Employ a simple round robin load distribution
    fn next_worker(&self) -> &PersistentDivStateStub<S> {
        let counter = self.round_robin_counter.fetch_add(1, Ordering::Relaxed);

        self.tx.get(counter % self.tx.len()).unwrap()
    }

    pub fn queue_descriptor(&self, descriptor: S::StateDescriptor) -> Result<()> {
        let state_message = DivisibleStateMessage::Descriptor(descriptor);

        self.next_worker().send(state_message)
            .context("Failed to queue descriptor")
    }

    pub fn queue_state_parts(&self, parts: Vec<Arc<ReadOnly<S::StatePart>>>) -> Result<()> {
        let state_message = DivisibleStateMessage::Parts(parts);

        self.next_worker().send(state_message)
            .context("Failed to queue state parts")
    }

    pub fn queue_descriptor_and_parts(
        &self,
        descriptor: S::StateDescriptor,
        parts: Vec<Arc<ReadOnly<S::StatePart>>>,
    ) -> Result<()> {
        let state_message = DivisibleStateMessage::PartsAndDescriptor(parts, descriptor);

        self.next_worker().send(state_message)
            .context("Failed to send state parts and descriptor message")
    }

    pub fn queue_delete_part(&self, part_descriptor: S::PartDescription) -> Result<()> {
        let state_message = DivisibleStateMessage::DeletePart(part_descriptor);

        self.next_worker().send(state_message)
            .context("Failed to send delete part message")
    }
}

#[allow(dead_code)]
pub struct DivStatePersistentLogWorker<S, RQ, OPM, POPT, LS, POP, PSP, DLPH>
where
    S: DivisibleState + 'static,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ> + 'static,
    POPT: PersistentOrderProtocolTypes<RQ, OPM> + 'static,
    LS: DecisionLogMessage<RQ, OPM, POPT> + 'static,
    POP: OrderProtocolLogHelper<RQ, OPM, POPT>,
    PSP: PersistableStateTransferProtocol + 'static,
    DLPH: DecisionLogPersistenceHelper<RQ, OPM, POPT, LS> + 'static,
{
    rx: ChannelSyncRx<DivisibleStateMessage<S>>,
    worker: PersistentLogWorker<RQ, OPM, POPT, LS, PSP, POP, DLPH>,
    db: KVDB,
}

#[allow(dead_code)]
impl<S, RQ, OPM, POPT, LS, POP, PSP, DLPH>
    DivStatePersistentLogWorker<S, RQ, OPM, POPT, LS, POP, PSP, DLPH>
where
    S: DivisibleState + 'static,
    RQ: SerMsg,
    OPM: OrderingProtocolMessage<RQ> + 'static,
    POPT: PersistentOrderProtocolTypes<RQ, OPM> + 'static,
    LS: DecisionLogMessage<RQ, OPM, POPT> + 'static,
    POP: OrderProtocolLogHelper<RQ, OPM, POPT>,
    PSP: PersistableStateTransferProtocol + 'static,
    DLPH: DecisionLogPersistenceHelper<RQ, OPM, POPT, LS> + 'static,
{
    pub fn new(
        request_rx: ChannelSyncRx<DivisibleStateMessage<S>>,
        inner_worker: PersistentLogWorker<RQ, OPM, POPT, LS, PSP, POP, DLPH>,
        db: KVDB,
    ) -> Result<Self> {
        Ok(Self {
            rx: request_rx,
            worker: inner_worker,
            db,
        })
    }

    pub fn work(mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(message) => {
                    let _result = quiet_unwrap!(self.exec_req(message));

                    // Try to receive more messages if possible
                    continue;
                }
                Err(err) => match err {
                    TryRecvError::ChannelEmpty => {}
                    TryRecvError::ChannelDc | TryRecvError::Timeout => {
                        error!("Error receiving message: {err:?}");
                    }
                },
            }

            if let Err(err) = self.worker.work_iteration() {
                error!("Failed to execute persistent log request because {err:?}");

                break;
            }
        }
    }

    fn exec_req(&mut self, message: DivisibleStateMessage<S>) -> Result<ResponseMessage> {
        Ok(match message {
            DivisibleStateMessage::Parts(part) => {
                write_state_parts::<S>(&self.db, &part)?;

                ResponseMessage::RegisteredCallback
            }
            DivisibleStateMessage::Descriptor(description) => {
                write_state_descriptor::<S>(&self.db, &description)?;

                ResponseMessage::RegisteredCallback
            }
            DivisibleStateMessage::PartsAndDescriptor(parts, description) => {
                write_state_parts_and_descriptor::<S>(&self.db, &parts, &description)?;

                ResponseMessage::RegisteredCallback
            }
            DivisibleStateMessage::DeletePart(_) => {
                todo!();
            }
        })
    }
}

impl<S> Deref for PersistentDivStateStub<S>
where
    S: DivisibleState,
{
    type Target = ChannelSyncTx<DivisibleStateMessage<S>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

const LATEST_STATE_DESCRIPTOR: &str = "latest_state_descriptor";

pub(crate) fn read_latest_descriptor<S: DivisibleState>(
    db: &KVDB,
) -> Result<Option<S::StateDescriptor>> {
    let result = db.get(COLUMN_FAMILY_STATE, LATEST_STATE_DESCRIPTOR)?;

    if let Some(descriptor) = result {
        let state_descriptor = deserialize_state_descriptor::<&[u8], S>(&mut descriptor.as_ref())?;

        Ok(Some(state_descriptor))
    } else {
        Ok(None)
    }
}

pub(crate) fn read_state_part<S: DivisibleState>(
    db: &KVDB,
    part: &S::PartDescription,
) -> Result<Option<S::StatePart>> {
    let mut key = Vec::new();

    serialize_state_part_descriptor::<Vec<u8>, S>(&mut key, part)?;

    let result = db.get(COLUMN_FAMILY_STATE, key)?;

    if let Some(value) = result {
        let state_part = deserialize_state_part::<&[u8], S>(&mut value.as_ref())?;

        Ok(Some(state_part))
    } else {
        Ok(None)
    }
}

fn write_state_parts<S: DivisibleState>(
    db: &KVDB,
    parts: &Vec<Arc<ReadOnly<S::StatePart>>>,
) -> Result<()> {
    for state_part in parts {
        let part_desc = state_part.descriptor();

        let mut key = Vec::new();

        serialize_state_part_descriptor::<Vec<u8>, S>(&mut key, &part_desc)?;

        let mut value = Vec::new();

        serialize_state_part::<Vec<u8>, S>(&mut value, &**state_part)?;

        db.set(COLUMN_FAMILY_STATE, key, value)?;
    }

    Ok(())
}

fn write_state_descriptor<S: DivisibleState>(
    db: &KVDB,
    descriptor: &S::StateDescriptor,
) -> Result<()> {
    let mut value = Vec::new();

    serialize_state_descriptor::<Vec<u8>, S>(&mut value, &descriptor)?;

    db.set(COLUMN_FAMILY_STATE, LATEST_STATE_DESCRIPTOR, &value)?;

    Ok(())
}

fn write_state_parts_and_descriptor<S: DivisibleState>(
    db: &KVDB,
    parts: &Vec<Arc<ReadOnly<S::StatePart>>>,
    descriptor: &S::StateDescriptor,
) -> Result<()> {
    write_state_parts::<S>(db, parts)?;
    write_state_descriptor::<S>(db, descriptor)?;

    Ok(())
}
