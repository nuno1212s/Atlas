use std::io::Read;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use log::error;
use atlas_common::error::*;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx, SendError, TryRecvError};
use atlas_common::crypto::hash::Digest;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::Orderable;
use atlas_common::persistentdb::KVDB;
use atlas_core::persistent_log::{PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_core::state_transfer::Checkpoint;
use atlas_execution::serialize::ApplicationData;
use atlas_execution::state::monolithic_state::MonolithicState;
use crate::{MonolithicStateMessage, ResponseMessage};
use crate::serialize::{deserialize_mon_state, make_seq, read_seq, serialize_mon_state};
use crate::worker::{COLUMN_FAMILY_STATE, PersistentLogWorker};

#[derive(Clone)]
pub struct PersistentMonolithicStateStub<S: MonolithicState> {
    pub(crate) tx: ChannelSyncTx<MonolithicStateMessage<S>>,
}

pub struct PersistentMonolithicStateHandle<S: MonolithicState> {
    round_robin_counter: AtomicUsize,
    tx: Vec<PersistentMonolithicStateStub<S>>,
}

impl<S> PersistentMonolithicStateHandle<S> where S: MonolithicState {
    pub(crate) fn new(tx: Vec<PersistentMonolithicStateStub<S>>) -> Self {
        Self {
            round_robin_counter: Default::default(),
            tx,
        }
    }

    /// Employ a simple round robin load distribution
    fn next_worker(&self) -> &PersistentMonolithicStateStub<S> {
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

    pub fn queue_state(&self, state: Arc<ReadOnly<Checkpoint<S>>>) -> Result<()> {
        let state_message = MonolithicStateMessage {
            checkpoint: state,
        };

        Self::translate_error(self.next_worker().send(state_message))
    }
}

pub struct MonStatePersistentLogWorker<S, D, OPM, SOPM, POP, PSP>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage + 'static,
          SOPM: StatefulOrderProtocolMessage + 'static,
          POP: PersistableOrderProtocol<OPM, SOPM> + 'static,
          PSP: PersistableStateTransferProtocol + 'static
{
    request_rx: ChannelSyncRx<MonolithicStateMessage<S>>,

    inner_worker: PersistentLogWorker<D, OPM, SOPM, POP, PSP>,

    db: KVDB,
}

impl<S, D, OPM, SOPM, POP, PSP> MonStatePersistentLogWorker<S, D, OPM, SOPM, POP, PSP>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage + 'static,
          SOPM: StatefulOrderProtocolMessage + 'static,
          POP: PersistableOrderProtocol<OPM, SOPM> + 'static,
          PSP: PersistableStateTransferProtocol + 'static
{
    pub fn new(request_rx: ChannelSyncRx<MonolithicStateMessage<S>>,
               inner_worker: PersistentLogWorker<D, OPM, SOPM, POP, PSP>,
               db: KVDB) -> Self {
        Self {
            request_rx,
            inner_worker,
            db,
        }
    }

    pub fn work(mut self) {
        loop {
            match self.request_rx.try_recv() {
                Ok(message) => {
                    let result = self.exec_req(message);

                    // Try to receive more messages if possible
                    continue;
                }
                Err(error_kind) => {
                    match error_kind {
                        TryRecvError::ChannelEmpty => {}
                        TryRecvError::ChannelDc | TryRecvError::Timeout => {
                            error!("Error receiving message: {:?}", error_kind);
                        }
                    }
                }
            }

            if let Err(err) = self.inner_worker.work_iteration() {
                error!("Failed to execute persistent log request because {:?}", err);

                break;
            }
        }
    }

    fn exec_req(&mut self, message: MonolithicStateMessage<S>) -> Result<ResponseMessage> {
        Ok({
            let MonolithicStateMessage {
                checkpoint,
            } = message;

            write_state(&self.db, checkpoint.state())?;

            ResponseMessage::Checkpointed(checkpoint.sequence_number())
        })
    }
}

impl<S> Deref for PersistentMonolithicStateStub<S> where S: MonolithicState {
    type Target = ChannelSyncTx<MonolithicStateMessage<S>>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

/// The keys for fast searching of the information
const LATEST_CHECKPOINT_KEY: &str = "latest_checkpoint";
const LATEST_CHECKPOINT_SEQ_NUM_KEY: &str = "latest_checkpoint_seq_num";
const LATEST_CHECKPOINT_DIGEST_KEY: &str = "latest_checkpoint_digest";

pub(crate) fn read_mon_state<S>(db: &KVDB) -> Result<Option<Checkpoint<S>>> where S: MonolithicState {
    let state = read_state::<S>(db)?;

    let seq_no = db.get(COLUMN_FAMILY_STATE, LATEST_CHECKPOINT_SEQ_NUM_KEY)?;
    let digest = db.get(COLUMN_FAMILY_STATE, LATEST_CHECKPOINT_DIGEST_KEY)?;

    match (seq_no, digest, state) {
        (Some(seq_no), Some(digest), Some(state)) => {
            let seq_no = read_seq(seq_no.as_slice())?;

            let digest = Digest::from_bytes(digest.as_slice())?;

            Ok(Some(Checkpoint::new_simple(seq_no, state, digest)))
        }
        _ => {
            Ok(None)
        }
    }
}

fn write_checkpoint<S>(db: &KVDB, state: Arc<ReadOnly<Checkpoint<S>>>) -> Result<()> where S: MonolithicState {
    let seq_no = make_seq(state.sequence_number())?;

    db.set(COLUMN_FAMILY_STATE, LATEST_CHECKPOINT_SEQ_NUM_KEY, seq_no.as_slice())?;

    db.set(COLUMN_FAMILY_STATE, LATEST_CHECKPOINT_DIGEST_KEY, state.digest())?;

    write_state::<S>(db, state.state())?;

    Ok(())
}

fn read_state<S>(db: &KVDB) -> Result<Option<S>> where S: MonolithicState {
    let serialized = db.get(COLUMN_FAMILY_STATE, LATEST_CHECKPOINT_KEY)?;

    let option = serialized.map(|serialized| {
        deserialize_mon_state::<&[u8], S>(&mut serialized.as_slice())
    });
    
    if let Some(result) = option {
        Ok(Some(result?))
    } else {
        Ok(None)
    }
}

fn write_state<S>(db: &KVDB, state: &S) -> Result<()> where S: MonolithicState {
    let mut serialized = Vec::new();

    serialize_mon_state::<Vec<u8>, S>(&mut serialized, state)?;

    db.set(COLUMN_FAMILY_STATE, LATEST_CHECKPOINT_KEY, serialized.as_slice())?;

    Ok(())
}