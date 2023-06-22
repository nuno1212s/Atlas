use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use log::error;
use atlas_common::error::*;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx, SendError, TryRecvError};
use atlas_common::persistentdb::KVDB;
use atlas_core::persistent_log::{PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_execution::serialize::ApplicationData;
use atlas_execution::state::divisible_state::{DivisibleState, StatePart};
use crate::{DivisibleStateMessage, ResponseMessage};
use crate::serialize::{serialize_state_descriptor, serialize_state_part, serialize_state_part_descriptor};
use crate::worker::{COLUMN_FAMILY_STATE, PersistentLogWorker};

#[derive(Clone)]
pub struct PersistentDivStateStub<S: DivisibleState> {
    pub(crate) tx: ChannelSyncTx<DivisibleStateMessage<S>>,
}

pub struct PersistentDivStateHandle<S: DivisibleState> {
    round_robin_counter: AtomicUsize,
    tx: Vec<PersistentDivStateStub<S>>,
}

impl<S> PersistentDivStateHandle<S> where S: DivisibleState {
    /// Employ a simple round robin load distribution
    fn next_worker(&self) -> &PersistentDivStateStub<S> {
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

    pub fn queue_descriptor(&self, descriptor: S::StateDescriptor) -> Result<()> {
        let state_message = DivisibleStateMessage::Descriptor(descriptor);

        Self::translate_error(self.next_worker().send(state_message))
    }

    pub fn queue_state_parts(&self, parts: Vec<S::StatePart>) -> Result<()> {
        let state_message = DivisibleStateMessage::StateParts(parts);

        Self::translate_error(self.next_worker().send(state_message))
    }

    pub fn queue_descriptor_and_parts(&self, descriptor: S::StateDescriptor, parts: Vec<S::StatePart>) -> Result<()> {
        let state_message = DivisibleStateMessage::DescriptorAndStateParts(descriptor, parts);

        Self::translate_error(self.next_worker().send(state_message))
    }
}

pub struct DivStatePersistentLogWorker<S, D, OPM, SOPM, POP, PSP>
    where S: DivisibleState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage + 'static,
          SOPM: StatefulOrderProtocolMessage + 'static,
          POP: PersistableOrderProtocol<OPM, SOPM> + 'static,
          PSP: PersistableStateTransferProtocol + 'static,
{
    rx: ChannelSyncRx<DivisibleStateMessage<S>>,
    worker: PersistentLogWorker<D, OPM, SOPM, POP, PSP>,
    db: KVDB,
}

impl<S, D, OPM, SOPM, POP, PSP> DivStatePersistentLogWorker<S, D, OPM, SOPM, POP, PSP>
    where S: DivisibleState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage + 'static,
          SOPM: StatefulOrderProtocolMessage + 'static,
          POP: PersistableOrderProtocol<OPM, SOPM> + 'static,
          PSP: PersistableStateTransferProtocol + 'static
{
    pub fn new(request_rx: ChannelSyncRx<DivisibleStateMessage<S>>,
               inner_worker: PersistentLogWorker<D, OPM, SOPM, POP, PSP>,
               db: KVDB) -> Self {
        Self {
            rx: request_rx,
            worker: inner_worker,
            db,
        }
    }

    pub fn work(mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(message) => {
                    let result = self.exec_req(message);

                    // Try to receive more messages if possible
                    continue
                }
                Err(err) => {
                    match err {
                        TryRecvError::ChannelEmpty => {}
                        TryRecvError::ChannelDc | TryRecvError::Timeout => {
                            error!("Error receiving message: {:?}", err);
                        }
                    }
                }
            }

            self.worker.work();
        }
    }

    fn exec_req(&mut self, message: DivisibleStateMessage<S>) -> Result<ResponseMessage> {
        Ok(
            match message {
                DivisibleStateMessage::Parts(part) => {
                    write_state_parts(&self.db, &part)?;

                    ResponseMessage::RegisteredCallback
                }
                DivisibleStateMessage::Descriptor(description) => {
                    write_state_descriptor(&self.db, &description)?;

                    ResponseMessage::RegisteredCallback
                }
                DivisibleStateMessage::PartsAndDescriptor(parts, description) => {
                    write_state_parts_and_descriptor(&self.db, &parts, &description)?;

                    ResponseMessage::RegisteredCallback
                }
            }
        )
    }
}

impl<S> Deref for PersistentDivStateStub<S> where S: DivisibleState {
    type Target = ChannelSyncTx<DivisibleStateMessage<S>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

const LATEST_STATE_DESCRIPTOR: &str = "latest_state_descriptor";

fn write_state_parts<S: DivisibleState>(
    db: &KVDB,
    parts: &Vec<S::StatePart>,
) -> Result<()> {
    for state_part in parts {
        let part_desc = state_part.descriptor();

        let mut key = Vec::new();

        serialize_state_part_descriptor::<Vec<u8>, S>(&mut key, &part_desc)?;

        let mut value = Vec::new();

        serialize_state_part::<Vec<u8>, S>(&mut value, &state_part)?;

        db.set(COLUMN_FAMILY_STATE, key, value)?;
    }

    Ok(())
}

fn write_state_descriptor<S: DivisibleState>(db: &KVDB, descriptor: &S::StateDescriptor)
                                             -> Result<()> {
    let mut value = Vec::new();

    serialize_state_descriptor(&mut value, &descriptor)?;

    db.set(COLUMN_FAMILY_STATE, LATEST_STATE_DESCRIPTOR, &value)?;

    Ok(())
}

fn write_state_parts_and_descriptor<S: DivisibleState>(
    db: &KVDB,
    parts: &Vec<S::StatePart>,
    descriptor: &S::StateDescriptor,
) -> Result<()> {
    write_state_parts(db, parts)?;
    write_state_descriptor(db, descriptor)?;

    Ok(())
}