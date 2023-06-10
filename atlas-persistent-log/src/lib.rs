use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use atlas_common::channel;
use atlas_common::channel::ChannelSyncTx;
use atlas_common::crypto::hash::Digest;
use atlas_execution::ExecutorHandle;
use atlas_execution::serialize::SharedData;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::persistentdb::KVDB;
use atlas_communication::message::StoredMessage;
use atlas_core::ordering_protocol::ProtocolConsensusDecision;
use atlas_core::persistent_log::{PersistableOrderProtocol, PSDecLog, PSMessage, PSProof, PSView};
use atlas_core::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_core::state_transfer::Checkpoint;
use crate::backlog::{ConsensusBacklog, ConsensusBackLogHandle};
use crate::worker::{COLUMN_FAMILY_OTHER, COLUMN_FAMILY_PROOFS, invalidate_seq, PersistentLogWorker, PersistentLogWorkerHandle, PersistentLogWriteStub, write_checkpoint, write_latest_seq_no, write_latest_view_seq_no, write_message, write_proof, write_proof_metadata, write_state};

pub mod serialize;
pub mod backlog;
mod worker;

/// The general type for a callback.
/// Callbacks are optional and can be used when you want to
/// execute a function when the logger stops finishes the computation
pub type CallbackType = Box<dyn FnOnce(Result<ResponseMessage>) + Send>;

pub enum PersistentLogMode<D: SharedData> {
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
            D: SharedData + 'static;
}

///Strict log mode initializer
pub struct StrictPersistentLog;

impl PersistentLogModeTrait for StrictPersistentLog {
    fn init_persistent_log<D>(executor: ExecutorHandle<D>) -> PersistentLogMode<D>
        where
            D: SharedData + 'static,
    {
        let handle = ConsensusBacklog::init_backlog(executor);

        PersistentLogMode::Strict(handle)
    }
}

///Optimistic log mode initializer
pub struct OptimisticPersistentLog;

impl PersistentLogModeTrait for OptimisticPersistentLog {
    fn init_persistent_log<D: SharedData + 'static>(_: ExecutorHandle<D>) -> PersistentLogMode<D> {
        PersistentLogMode::Optimistic
    }
}

pub struct NoPersistentLog;

impl PersistentLogModeTrait for NoPersistentLog {
    fn init_persistent_log<D>(_: ExecutorHandle<D>) -> PersistentLogMode<D> where D: SharedData + 'static {
        PersistentLogMode::None
    }
}

///How should the data be written and response delivered?
/// If Sync is chosen the function will block on the call and return the result of the operation
/// If Async is chosen the function will not block and will return the response as a message to a channel
pub enum WriteMode {
    //When writing in async mode, you have the option of having the response delivered on a function
    //Of your choice
    //Note that this function will be executed on the persistent logging thread, so keep it short and
    //Be careful with race conditions.
    NonBlockingSync(Option<CallbackType>),
    BlockingSync,
}


///TODO: Handle sequence numbers that loop the u32 range.
/// This is the main reference to the persistent log, used to push data to it
pub struct PersistentLog<D: SharedData, PS: PersistableOrderProtocol>
{
    persistency_mode: PersistentLogMode<D>,

    // A handle for the persistent log workers (each with his own thread)
    worker_handle: Arc<PersistentLogWorkerHandle<D, PS>>,

    ///The persistent KV-DB to be used
    db: KVDB,
}

/// The type of the installed state information
pub type InstallState<D: SharedData, PS: PersistableOrderProtocol> = (
    //The view sequence number
    SeqNo,
    //The decision log that comes after that state
    PSDecLog<PS>,
);

/// Work messages for the
pub(crate) enum PWMessage<D: SharedData, PS: PersistableOrderProtocol> {
    //Persist a new view into the persistent storage
    View(PSView<PS>),

    //Persist a new sequence number as the consensus instance has been committed and is therefore ready to be persisted
    Committed(SeqNo),

    // Persist the metadata for a given decision
    ProofMetadata(PS::ProofMetadata),

    //Persist a given message into storage
    Message(Arc<ReadOnly<StoredMessage<PSMessage<PS>>>>),

    //Persist a given state into storage.
    Checkpoint(Arc<ReadOnly<Checkpoint<D::State>>>),

    //Remove all associated stored messages for this given seq number
    Invalidate(SeqNo),

    // Register a proof of the decision log
    Proof(PSProof<PS>),

    //Install a recovery state received from CST or produced by us
    InstallState(InstallState<D, PS>),

    /// Register a new receiver for messages sent by the persistency workers
    RegisterCallbackReceiver(ChannelSyncTx<ResponseMessage>),
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

    // Stored the proof with the given sequence
    Proof(SeqNo),

    RegisteredCallback,
}

/// Messages that are sent to the logging thread to log specific requests
pub(crate) type ChannelMsg<D: SharedData, PS: PersistableOrderProtocol> = (PWMessage<D, PS>, Option<CallbackType>);

pub fn initialize_persistent_log<D, K, T, PS>(executor: ExecutorHandle<D>, db_path: K)
                                          -> Result<PersistentLog<D, PS>>
    where D: SharedData + 'static, K: AsRef<Path>, T: PersistentLogModeTrait, PS: PersistableOrderProtocol {
    PersistentLog::init_log::<K, T>(executor, db_path)
}

impl<D, PS> PersistentLog<D, PS> where D: SharedData, PS: PersistableOrderProtocol {
    fn init_log<K, T>(executor: ExecutorHandle<D>, db_path: K) -> Result<Self>
        where
            K: AsRef<Path>,
            T: PersistentLogModeTrait
    {
        let mut message_types = PS::message_types();

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

        let worker = PersistentLogWorker::new(rx, response_txs, kvdb.clone());

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
            db: kvdb,
        })
    }

    /// TODO: Maybe make this async? We need it to start execution anyways...
    pub fn read_state(&self) -> Result<Option<InstallState<D, PS>>> {
        match self.kind() {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                read_latest_state::<D>(&self.db)
            }
            PersistentLogMode::None => {
                Ok(None)
            }
        }
    }

    pub fn kind(&self) -> &PersistentLogMode<D> {
        &self.persistency_mode
    }

    pub fn write_committed_seq_no(&self, write_mode: WriteMode, seq: SeqNo) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    WriteMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_committed(seq, callback)
                    }
                    WriteMode::BlockingSync => write_latest_seq_no(&self.db, seq),
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    pub fn write_view_info(&self, write_mode: WriteMode, view_seq: PSView<PS>) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    WriteMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_view_number(view_seq, callback)
                    }
                    WriteMode::BlockingSync => write_latest_view_seq_no(&self.db, view_seq.sequence_number()),
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    /// Write the metadata of the proof
    pub fn write_proof_metadata(&self, write_mode: WriteMode,
                                metadata: PS::ProofMetadata) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    WriteMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_proof_metadata(metadata, callback)
                    }
                    WriteMode::BlockingSync => {
                        write_proof_metadata(&self.db, metadata)
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    pub fn write_message(
        &self,
        write_mode: WriteMode,
        msg: Arc<ReadOnly<StoredMessage<PSMessage<PS>>>>,
    ) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    WriteMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_message(msg, callback)
                    }
                    WriteMode::BlockingSync => write_message::<D, PS>(&self.db, &msg),
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    pub fn write_checkpoint(
        &self,
        write_mode: WriteMode,
        checkpoint: Arc<ReadOnly<Checkpoint<D::State>>>,
    ) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    WriteMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_state(checkpoint, callback)
                    }
                    WriteMode::BlockingSync => {

                        write_checkpoint::<D, PS>(&self.db, checkpoint)
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    pub fn write_invalidate(&self, write_mode: WriteMode, seq: SeqNo) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    WriteMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_invalidate(seq, callback)
                    }
                    WriteMode::BlockingSync => {
                        invalidate_seq(&self.db, seq)?;

                        Ok(())
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    /// Attempt to install the state into persistent storage
    pub fn write_install_state(&self, write_mode: WriteMode, state: InstallState<D, PS>) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    WriteMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_install_state(state, callback)
                    }
                    WriteMode::BlockingSync => {
                        write_state::<D, PS>(&self.db, state)
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
    }

    /// Write a proof to the persistent log.
    pub fn write_proof(&self, write_mode: WriteMode, proof: PSProof<PS>) -> Result<()> {
        match self.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                match write_mode {
                    WriteMode::NonBlockingSync(callback) => {
                        self.worker_handle.queue_proof(proof, callback)
                    }
                    WriteMode::BlockingSync => {
                        write_proof::<D, PS>(&self.db, proof)
                    }
                }
            }
            PersistentLogMode::None => {
                Ok(())
            }
        }
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

impl<D: SharedData, PS: PersistableOrderProtocol> Clone for PersistentLog<D, PS> {
    fn clone(&self) -> Self {
        Self {
            persistency_mode: self.persistency_mode.clone(),
            worker_handle: self.worker_handle.clone(),
            db: self.db.clone(),
        }
    }
}

impl<D: SharedData> Clone for PersistentLogMode<D> {
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


