extern crate core;

use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use atlas_common::channel;
use atlas_common::channel::ChannelSyncTx;
use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::persistentdb::KVDB;
use atlas_communication::message::StoredMessage;
use atlas_core::ordering_protocol::{LoggableMessage, ProtocolConsensusDecision, SerProof, SerProofMetadata, View};
use atlas_core::ordering_protocol::networking::serialize::{OrderingProtocolMessage, PermissionedOrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_core::ordering_protocol::stateful_order_protocol::DecLog;
use atlas_core::persistent_log::{DivisibleStateLog, MonolithicStateLog, OperationMode, OrderingProtocolLog, PersistableOrderProtocol, PersistableStateTransferProtocol, StatefulOrderingProtocolLog};
use atlas_core::state_transfer::Checkpoint;
use atlas_core::state_transfer::networking::serialize::StateTransferMessage;
use atlas_execution::ExecutorHandle;
use atlas_execution::serialize::ApplicationData;
use atlas_execution::state::divisible_state::DivisibleState;
use atlas_execution::state::monolithic_state::MonolithicState;

use crate::backlog::{ConsensusBacklog, ConsensusBackLogHandle};
use crate::worker::{COLUMN_FAMILY_OTHER, COLUMN_FAMILY_PROOFS, PersistentLogWorker, PersistentLogWorkerHandle, PersistentLogWriteStub, write_latest_seq_no};
use crate::worker::divisible_state_worker::{DivStatePersistentLogWorker, PersistentDivStateHandle, PersistentDivStateStub};
use crate::worker::monolithic_worker::{MonStatePersistentLogWorker, PersistentMonolithicStateHandle, PersistentMonolithicStateStub, read_mon_state};

pub mod serialize;
pub mod backlog;
mod worker;
pub mod metrics;

/// The general type for a callback.
/// Callbacks are optional and can be used when you want to
/// execute a function when the logger stops finishes the computation
// pub type CallbackType = Box<dyn FnOnce(Result<ResponseMessage>) + Send>;
pub type CallbackType = ();

pub enum PersistentLogMode<D: ApplicationData> {
    /// The strict log mode is meant to indicate that the consensus can only be finalized and the
    /// requests executed when the replica has all the information persistently stored.
    ///
    /// This allows for all replicas to crash and still be able to recover from their own stored
    /// local state, meaning we can always recover without losing any piece of replied to information
    /// So we have the guarantee that once a request has been replied to, it will never be lost (given f byzantine faults).
    ///
    /// Performance will be dependent on the speed of the datastore as the consensus will only move to the
    /// executing phase once all requests have been successfully stored.
    Strict(ConsensusBackLogHandle<D::Request>),

    /// Optimistic mode relies a lot more on the assumptions that are made by the BFT algorithm in order
    /// to maximize the performance.
    ///
    /// It works by separating the persistent data storage with the consensus algorithm. It relies on
    /// the fact that we only allow for f faults concurrently, so we assume that we can never have a situation
    /// where more than f replicas fail at the same time, so they can always rely on the existence of other
    /// replicas that it can use to rebuild it's state from where it left off.
    ///
    /// One might say this provides no security benefits comparatively to storing information just in RAM (since
    /// we don't have any guarantees on what was actually stored in persistent storage)
    /// however this does provide more performance benefits as we don't have to rebuild the entire state from the
    /// other replicas of the system, which would degrade performance. We can take our incomplete state and
    /// just fill in the blanks using the state transfer algorithm
    Optimistic,

    /// Perform no persistent logging to the database and rely only on the prospect that
    /// We are always able to rebuild our state from other replicas that may be online
    None,
}

pub trait PersistentLogModeTrait: Send {
    fn init_persistent_log<D>(executor: ExecutorHandle<D>) -> PersistentLogMode<D>
        where
            D: ApplicationData + 'static;
}

///Strict log mode initializer
pub struct StrictPersistentLog;

impl PersistentLogModeTrait for StrictPersistentLog {
    fn init_persistent_log<D>(executor: ExecutorHandle<D>) -> PersistentLogMode<D>
        where
            D: ApplicationData + 'static,
    {
        let handle = ConsensusBacklog::init_backlog(executor);

        PersistentLogMode::Strict(handle)
    }
}

///Optimistic log mode initializer
pub struct OptimisticPersistentLog;

impl PersistentLogModeTrait for OptimisticPersistentLog {
    fn init_persistent_log<D: ApplicationData + 'static>(_: ExecutorHandle<D>) -> PersistentLogMode<D> {
        PersistentLogMode::Optimistic
    }
}

pub struct NoPersistentLog;

impl PersistentLogModeTrait for NoPersistentLog {
    fn init_persistent_log<D>(_: ExecutorHandle<D>) -> PersistentLogMode<D> where D: ApplicationData + 'static {
        PersistentLogMode::None
    }
}

///TODO: Handle sequence numbers that loop the u32 range.
/// This is the main reference to the persistent log, used to push data to it
pub struct PersistentLog<D: ApplicationData,
    OPM: OrderingProtocolMessage<D>,
    SOPM: StatefulOrderProtocolMessage<D, OPM>,
    POP: PermissionedOrderingProtocolMessage,
    STM: StateTransferMessage>
{
    persistency_mode: PersistentLogMode<D>,

    // A handle for the persistent log workers (each with his own thread)
    worker_handle: Arc<PersistentLogWorkerHandle<D, OPM, SOPM, POP>>,

    p: PhantomData<STM>,
    ///The persistent KV-DB to be used
    db: KVDB,
}

/// The persistent log handle to the worker for the monolithic state persistency log
pub struct MonStatePersistentLog<S: MonolithicState, D: ApplicationData,
    OPM: OrderingProtocolMessage<D>,
    SOPM: StatefulOrderProtocolMessage<D, OPM>,
    POP: PermissionedOrderingProtocolMessage,
    STM: StateTransferMessage> {
    request_tx: Arc<PersistentMonolithicStateHandle<S>>,

    inner_log: PersistentLog<D, OPM, SOPM, POP, STM>,
}


pub struct DivisibleStatePersistentLog<S: DivisibleState, D: ApplicationData,
    OPM: OrderingProtocolMessage<D>, SOPM: StatefulOrderProtocolMessage<D, OPM>,
    POP: PermissionedOrderingProtocolMessage, STM: StateTransferMessage> {
    request_tx: Arc<PersistentDivStateHandle<S>>,

    inner_log: PersistentLog<D, OPM, SOPM, POP, STM>,
}

/// The type of the installed state information
pub type InstallState<D, OPM: OrderingProtocolMessage<D>, SOPM: StatefulOrderProtocolMessage<D, OPM>, POP: PermissionedOrderingProtocolMessage> = (
    //The view sequence number
    View<POP>,
    //The decision log that comes after that state
    DecLog<D, OPM, SOPM>,
);

/// Work messages for the persistent log workers
pub enum PWMessage<D, OPM: OrderingProtocolMessage<D>, SOPM: StatefulOrderProtocolMessage<D, OPM>, POP: PermissionedOrderingProtocolMessage> {
    //Persist a new view into the persistent storage
    View(View<POP>),

    //Persist a new sequence number as the consensus instance has been committed and is therefore ready to be persisted
    Committed(SeqNo),

    // Persist the metadata for a given decision
    ProofMetadata(SerProofMetadata<D, OPM>),

    //Persist a given message into storage
    Message(Arc<ReadOnly<StoredMessage<LoggableMessage<D, OPM>>>>),

    //Remove all associated stored messages for this given seq number
    Invalidate(SeqNo),

    // Register a proof of the decision log
    Proof(SerProof<D, OPM>),

    //Install a recovery state received from CST or produced by us
    InstallState(InstallState<D, OPM, SOPM, POP>),

    /// Register a new receiver for messages sent by the persistency workers
    RegisterCallbackReceiver(ChannelSyncTx<ResponseMessage>),
}

/// The message containing the information necessary to persist the most recently received
/// Monolithic state
pub struct MonolithicStateMessage<S: MonolithicState> {
    checkpoint: Arc<ReadOnly<Checkpoint<S>>>,
}

/// The message containing the information necessary to persist the most recently received
/// State parts
pub enum DivisibleStateMessage<S: DivisibleState> {
    Parts(Vec<Arc<ReadOnly<S::StatePart>>>),
    Descriptor(S::StateDescriptor),
    PartsAndDescriptor(Vec<Arc<ReadOnly<S::StatePart>>>, S::StateDescriptor),
    DeletePart(S::PartDescription),
}

/// Messages sent by the persistency workers to notify the registered receivers
#[derive(Clone)]
pub enum ResponseMessage {
    ///Notify that we have persisted the view with the given sequence number
    ViewPersisted(SeqNo),

    ///Notifies that we have persisted the sequence number that has been persisted (Only the actual sequence number)
    /// Not related to actually persisting messages
    CommittedPersisted(SeqNo),

    // Notifies that the metadata for a given seq no has been persisted
    WroteMetadata(SeqNo),

    ///Notifies that a message with a given SeqNo and a given unique identifier for the message
    /// TODO: Decide this unique identifier
    WroteMessage(SeqNo, Digest),

    // Notifies that the state has been successfully installed and returns
    InstalledState(SeqNo),

    /// Notifies that all messages relating to the given sequence number have been destroyed
    InvalidationPersisted(SeqNo),

    /// Notifies that the given checkpoint was persisted into the database
    Checkpointed(SeqNo),
    /*
    WroteParts(Vec<Digest>),

    WroteDescriptor(SeqNo),

    WrotePartsAndDescriptor(SeqNo, Vec<Digest>),*/

    // Stored the proof with the given sequence
    Proof(SeqNo),

    RegisteredCallback,
}

/// Messages that are sent to the logging thread to log specific requests
pub(crate) type ChannelMsg<D, OPM: OrderingProtocolMessage<D>,
    SOPM: StatefulOrderProtocolMessage<D, OPM>, POP: PermissionedOrderingProtocolMessage> = (PWMessage<D, OPM, SOPM, POP>, Option<CallbackType>);

pub fn initialize_mon_persistent_log<S, D, K, T, OPM, SOPM, POPM, STM, POP, PSP>(executor: ExecutorHandle<D>, db_path: K)
                                                                                 -> Result<MonStatePersistentLog<S, D, OPM, SOPM, POPM, STM>>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          K: AsRef<Path>,
          T: PersistentLogModeTrait,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POPM: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static,
          POP: PersistableOrderProtocol<D, OPM, SOPM> + Send + 'static,
          PSP: PersistableStateTransferProtocol + Send + 'static
{
    MonStatePersistentLog::init_mon_log::<K, T, POP, PSP>(executor, db_path)
}

impl<D, OPM, SOPM, POP, STM> PersistentLog<D, OPM, SOPM, POP, STM>
    where D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static,
{
    fn init_log<K, T, POS, PSP>(executor: ExecutorHandle<D>, db_path: K) -> Result<Self>
        where
            K: AsRef<Path>,
            T: PersistentLogModeTrait,
            POS: PersistableOrderProtocol<D, OPM, SOPM> + Send + 'static,
            PSP: PersistableStateTransferProtocol + Send + 'static
    {
        let mut message_types = POS::message_types();

        let mut prefixes = vec![COLUMN_FAMILY_OTHER, COLUMN_FAMILY_PROOFS];

        prefixes.append(&mut message_types);

        let log_mode = T::init_persistent_log(executor);

        let mut response_txs = vec![];

        match &log_mode {
            PersistentLogMode::Strict(handle) => response_txs.push(handle.logger_tx().clone()),
            _ => {}
        }

        let kvdb = KVDB::new(db_path, prefixes)?;

        let (tx, rx) = channel::new_bounded_sync(1024);

        let worker = PersistentLogWorker::<D, OPM, SOPM, POP, POS, PSP>::new(rx, response_txs, kvdb.clone());

        match &log_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                std::thread::Builder::new().name(format!("Persistent log Worker #1"))
                    .spawn(move || {
                        worker.work();
                    }).unwrap();
            }
            _ => {}
        }

        let persistent_log_write_stub = PersistentLogWriteStub { tx };

        let worker_handle = Arc::new(PersistentLogWorkerHandle::new(vec![persistent_log_write_stub]));

        Ok(Self {
            persistency_mode: log_mode,
            worker_handle,
            p: Default::default(),
            db: kvdb,
        })
    }

    pub fn kind(&self) -> &PersistentLogMode<D> {
        &self.persistency_mode
    }

    ///Attempt to queue a batch into waiting for persistent logging
    /// If the batch does not have to wait, it's returned to it can be instantly
    /// passed to the executor
    pub fn wait_for_batch_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>> {
        match &self.persistency_mode {
            PersistentLogMode::Strict(consensus_backlog) => {
                consensus_backlog.queue_batch(batch)?;

                Ok(None)
            }
            PersistentLogMode::Optimistic | PersistentLogMode::None => {
                Ok(Some(batch))
            }
        }
    }

    ///Attempt to queue a batch that was received in the form of a completed proof
    /// into waiting for persistent logging, instead of receiving message by message (Received in
    /// a view change)
    /// If the batch does not have to wait, it's returned to it can be instantly
    /// passed to the executor
    pub fn wait_for_proof_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>> {
        match &self.persistency_mode {
            PersistentLogMode::Strict(backlog) => {
                backlog.queue_batch_proof(batch)?;

                Ok(None)
            }
            _ => {
                Ok(Some(batch))
            }
        }
    }
}

impl<D, OPM, SOPM, POP, STM> OrderingProtocolLog<D, OPM> for PersistentLog<D, OPM, SOPM, POP, STM>
    where D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          POP: PermissionedOrderingProtocolMessage,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          STM: StateTransferMessage + 'static {
    #[inline]
    fn write_committed_seq_no(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_committed(seq, callback)
                    }
                    OperationMode::BlockingSync => write_latest_seq_no(&self.db, seq),
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }


    #[inline]
    fn write_message(&self, write_mode: OperationMode, msg: Arc<ReadOnly<StoredMessage<LoggableMessage<D, OPM>>>>) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_message(msg, callback)
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    #[inline]
    fn write_proof_metadata(&self, write_mode: OperationMode, metadata: SerProofMetadata<D, OPM>) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_proof_metadata(metadata, callback)
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    #[inline]
    fn write_proof(&self, write_mode: OperationMode, proof: SerProof<D, OPM>) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_proof(proof, callback)
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    #[inline]
    fn write_invalidate(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_invalidate(seq, callback)
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }
}

impl<D, OPM, SOPM, POP, STM> StatefulOrderingProtocolLog<D, OPM, SOPM, POP> for PersistentLog<D, OPM, SOPM, POP, STM>
    where D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static {
    #[inline]
    fn write_view_info(&self, write_mode: OperationMode, view_seq: View<POP>) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_view_number(view_seq, callback)
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    #[inline]
    fn read_state(&self, write_mode: OperationMode) -> Result<Option<(View<POP>, DecLog<D, OPM, SOPM>)>> {
        match self.kind() {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                todo!();
                // let option = read_latest_state::<OPM, SOPM, PS>(&self.db)?;
                //
                // return if let Some((view, dec_log)) = option {
                //     Ok(Some((view, dec_log)))
                // } else {
                //     Ok(None)
                // };
            }
            PersistentLogMode::None => {
                Ok(None)
            }
        }
    }

    #[inline]
    fn write_install_state(&self, write_mode: OperationMode, view: View<POP>, dec_log: DecLog<D, OPM, SOPM>) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_install_state((view, dec_log), callback)
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }
}

impl<S, D, OPM, SOPM, POP, STM> MonStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static {
    fn init_mon_log<K, T, POS, PSP>(executor: ExecutorHandle<D>, db_path: K) -> Result<Self>
        where
            K: AsRef<Path>,
            T: PersistentLogModeTrait,
            POS: PersistableOrderProtocol<D, OPM, SOPM> + Send + 'static,
            PSP: PersistableStateTransferProtocol + Send + 'static {
        let mut message_types = POS::message_types();

        let mut prefixes = vec![COLUMN_FAMILY_OTHER, COLUMN_FAMILY_PROOFS];

        prefixes.append(&mut message_types);

        let log_mode = T::init_persistent_log(executor);

        let mut response_txs = vec![];

        match &log_mode {
            PersistentLogMode::Strict(handle) => response_txs.push(handle.logger_tx().clone()),
            _ => {}
        }

        let kvdb = KVDB::new(db_path, prefixes)?;

        let (tx, rx) = channel::new_bounded_sync(1024);

        let worker = PersistentLogWorker::<D, OPM, SOPM, POP, POS, PSP>::new(rx, response_txs, kvdb.clone());

        let (state_tx, state_rx) = channel::new_bounded_sync(10);

        let worker = MonStatePersistentLogWorker::<S, D, OPM, SOPM, POP, POS, PSP>::new(state_rx, worker, kvdb.clone());

        match &log_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                std::thread::Builder::new().name(format!("Persistent log Worker #1"))
                    .spawn(move || {
                        worker.work();
                    }).unwrap();
            }
            _ => {}
        }

        let persistent_log_write_stub = PersistentLogWriteStub { tx };

        let worker_handle = Arc::new(PersistentLogWorkerHandle::new(vec![persistent_log_write_stub]));

        let init_log = PersistentLog {
            persistency_mode: log_mode,
            worker_handle,
            p: Default::default(),
            db: kvdb,
        };

        let persistent_mon_write_stub = PersistentMonolithicStateStub { tx: state_tx };

        let worker_handle = Arc::new(PersistentMonolithicStateHandle::new(vec![persistent_mon_write_stub]));

        Ok(Self {
            request_tx: worker_handle,
            inner_log: init_log,
        })
    }

    /// Redirection to the inner log
    #[inline]
    pub fn wait_for_batch_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>> {
        self.inner_log.wait_for_batch_persistency_and_execute(batch)
    }


    #[inline]
    pub fn wait_for_proof_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>> {
        self.inner_log.wait_for_proof_persistency_and_execute(batch)
    }
}

impl<S, D, OPM, SOPM, POP, STM> MonolithicStateLog<S> for MonStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static {
    #[inline]
    fn read_checkpoint(&self) -> Result<Option<Checkpoint<S>>> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                read_mon_state(&self.inner_log.db)
            }
            PersistentLogMode::None => {
                Ok(None)
            }
        }
    }

    #[inline]
    fn write_checkpoint(&self, write_mode: OperationMode, checkpoint: Arc<ReadOnly<Checkpoint<S>>>) -> Result<()> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.request_tx.queue_state(checkpoint).unwrap();
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {}
        }

        Ok(())
    }
}

impl<S, D, OPM, SOPM, POP, STM> OrderingProtocolLog<D, OPM> for MonStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static {
    #[inline]
    fn write_committed_seq_no(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()> {
        self.inner_log.write_committed_seq_no(write_mode, seq)
    }

    #[inline]
    fn write_message(&self, write_mode: OperationMode, msg: Arc<ReadOnly<StoredMessage<LoggableMessage<D, OPM>>>>) -> Result<()> {
        self.inner_log.write_message(write_mode, msg)
    }

    #[inline]
    fn write_proof_metadata(&self, write_mode: OperationMode, metadata: SerProofMetadata<D, OPM>) -> Result<()> {
        self.inner_log.write_proof_metadata(write_mode, metadata)
    }

    #[inline]
    fn write_proof(&self, write_mode: OperationMode, proof: SerProof<D, OPM>) -> Result<()> {
        self.inner_log.write_proof(write_mode, proof)
    }

    #[inline]
    fn write_invalidate(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()> {
        self.inner_log.write_invalidate(write_mode, seq)
    }
}

impl<S, D, OPM, SOPM, POP, STM> StatefulOrderingProtocolLog<D, OPM, SOPM, POP> for MonStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static {
    #[inline]
    fn write_view_info(&self, write_mode: OperationMode, view_seq: View<POP>) -> Result<()> {
        self.inner_log.write_view_info(write_mode, view_seq)
    }

    fn read_state(&self, write_mode: OperationMode) -> Result<Option<(View<POP>, DecLog<D, OPM, SOPM>)>> {
        self.inner_log.read_state(write_mode)
    }

    fn write_install_state(&self, write_mode: OperationMode, view: View<POP>, dec_log: DecLog<D, OPM, SOPM>) -> Result<()> {
        self.inner_log.write_install_state(write_mode, view, dec_log)
    }
}


impl<S, D, OPM, SOPM, POP, STM> DivisibleStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: DivisibleState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static
{
    fn init_div_log<K, T, POS, PSP>(executor: ExecutorHandle<D>, db_path: K) -> Result<Self>
        where
            K: AsRef<Path>,
            T: PersistentLogModeTrait,
            POS: PersistableOrderProtocol<D, OPM, SOPM> + Send + 'static,
            PSP: PersistableStateTransferProtocol + Send + 'static {
        let mut message_types = POS::message_types();

        let mut prefixes = vec![COLUMN_FAMILY_OTHER, COLUMN_FAMILY_PROOFS];

        prefixes.append(&mut message_types);

        let log_mode = T::init_persistent_log(executor);

        let mut response_txs = vec![];

        match &log_mode {
            PersistentLogMode::Strict(handle) => response_txs.push(handle.logger_tx().clone()),
            _ => {}
        }

        let kvdb = KVDB::new(db_path, prefixes)?;

        let (tx, rx) = channel::new_bounded_sync(1024);

        let worker = PersistentLogWorker::<D, OPM, SOPM, POP, POS, PSP>::new(rx, response_txs, kvdb.clone());

        let (state_tx, state_rx) = channel::new_bounded_sync(10);

        let worker = DivStatePersistentLogWorker::<S, D, OPM, SOPM, POP, POS, PSP>::new(state_rx, worker, kvdb.clone())?;

        match &log_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                std::thread::Builder::new().name(format!("Persistent log Worker #1"))
                    .spawn(move || {
                        worker.work();
                    }).unwrap();
            }
            _ => {}
        }

        let persistent_log_write_stub = PersistentLogWriteStub { tx };

        let worker_handle = Arc::new(PersistentLogWorkerHandle::new(vec![persistent_log_write_stub]));

        let init_log = PersistentLog {
            persistency_mode: log_mode,
            worker_handle,
            p: Default::default(),
            db: kvdb,
        };

        let persistent_div_state = PersistentDivStateStub { tx: state_tx };

        let worker_handle = Arc::new(PersistentDivStateHandle::new(vec![persistent_div_state]));

        Ok(Self {
            request_tx: worker_handle,
            inner_log: init_log,
        })
    }
}

impl<S, D, OPM, SOPM, POP, STM> OrderingProtocolLog<D, OPM> for DivisibleStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: DivisibleState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static
{
    fn write_committed_seq_no(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()> {
        self.inner_log.write_committed_seq_no(write_mode, seq)
    }

    fn write_message(&self, write_mode: OperationMode, msg: Arc<ReadOnly<StoredMessage<LoggableMessage<D, OPM>>>>) -> Result<()> {
        self.inner_log.write_message(write_mode, msg)
    }

    fn write_proof_metadata(&self, write_mode: OperationMode, metadata: SerProofMetadata<D, OPM>) -> Result<()> {
        self.inner_log.write_proof_metadata(write_mode, metadata)
    }

    fn write_proof(&self, write_mode: OperationMode, proof: SerProof<D, OPM>) -> Result<()> {
        self.inner_log.write_proof(write_mode, proof)
    }

    fn write_invalidate(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()> {
        self.inner_log.write_invalidate(write_mode, seq)
    }
}

impl<S, D, OPM, SOPM, POP, STM> StatefulOrderingProtocolLog<D, OPM, SOPM, POP> for DivisibleStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: DivisibleState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static
{
    fn write_view_info(&self, write_mode: OperationMode, view_seq: View<POP>) -> Result<()> {
        self.inner_log.write_view_info(write_mode, view_seq)
    }

    fn read_state(&self, write_mode: OperationMode) -> Result<Option<(View<POP>, DecLog<D, OPM, SOPM>)>> {
        self.inner_log.read_state(write_mode)
    }

    fn write_install_state(&self, write_mode: OperationMode, view: View<POP>, dec_log: DecLog<D, OPM, SOPM>) -> Result<()> {
        self.inner_log.write_install_state(write_mode, view, dec_log)
    }
}

impl<S, D, OPM, SOPM, POP, STM> DivisibleStateLog<S> for DivisibleStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: DivisibleState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static
{
    fn write_descriptor(&self, write_mode: OperationMode, checkpoint: S::StateDescriptor) -> Result<()> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.request_tx.queue_descriptor(checkpoint)?;
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {}
        }

        Ok(())
    }

    fn delete_part(&self, write_mode: OperationMode, part: S::PartDescription) -> Result<()> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.request_tx.queue_delete_part(part)?;
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {}
        }

        Ok(())
    }

    fn write_parts(&self, write_mode: OperationMode, parts: Vec<Arc<ReadOnly<S::StatePart>>>) -> Result<()> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.request_tx.queue_state_parts(parts).unwrap();
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {}
        }

        Ok(())
    }

    fn write_parts_and_descriptor(&self, write_mode: OperationMode, descriptor: S::StateDescriptor, parts: Vec<Arc<ReadOnly<S::StatePart>>>) -> Result<()> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    OperationMode::NonBlockingSync(callback) => {
                        self.request_tx.queue_descriptor_and_parts(descriptor, parts).unwrap();
                    }
                    OperationMode::BlockingSync => {
                        todo!()
                    }
                }
            }
            PersistentLogMode::None => {}
        }

        Ok(())
    }

    fn read_local_descriptor(&self) -> Result<Option<S::StateDescriptor>> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                worker::divisible_state_worker::read_latest_descriptor::<S>(&self.inner_log.db)
            }
            PersistentLogMode::None => {
                Ok(None)
            }
        }
    }

    fn read_local_part(&self, part: S::PartDescription) -> Result<Option<S::StatePart>> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                worker::divisible_state_worker::read_state_part::<S>(&self.inner_log.db, &part)
            }
            PersistentLogMode::None => {
                Ok(None)
            }
        }
    }
}

impl<S, D, OPM, SOPM, POP, STM> Clone for MonStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static {
    fn clone(&self) -> Self {
        Self {
            request_tx: self.request_tx.clone(),
            inner_log: self.inner_log.clone(),
        }
    }
}

impl<S, D, OPM, SOPM, POP, STM> Clone for DivisibleStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: DivisibleState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          STM: StateTransferMessage + 'static {
    fn clone(&self) -> Self {
        Self {
            request_tx: self.request_tx.clone(),
            inner_log: self.inner_log.clone(),
        }
    }
}

impl<D: ApplicationData, OPM: OrderingProtocolMessage<D>, SOPM: StatefulOrderProtocolMessage<D, OPM>,
    POP: PermissionedOrderingProtocolMessage, STM: StateTransferMessage> Clone for PersistentLog<D, OPM, SOPM, POP, STM> {
    fn clone(&self) -> Self {
        Self {
            persistency_mode: self.persistency_mode.clone(),
            worker_handle: self.worker_handle.clone(),
            p: Default::default(),
            db: self.db.clone(),
        }
    }
}

impl<D: ApplicationData> Clone for PersistentLogMode<D> {
    fn clone(&self) -> Self {
        match self {
            PersistentLogMode::Strict(handle) => {
                PersistentLogMode::Strict(handle.clone())
            }
            PersistentLogMode::Optimistic => {
                PersistentLogMode::Optimistic
            }
            PersistentLogMode::None => {
                PersistentLogMode::None
            }
        }
    }
}


