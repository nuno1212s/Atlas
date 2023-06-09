use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use atlas_common::channel::ChannelSyncTx;
use atlas_common::crypto::hash::Digest;
use atlas_execution::ExecutorHandle;
use atlas_execution::serialize::SharedData;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::SeqNo;
use atlas_common::persistentdb::KVDB;
use atlas_communication::message::StoredMessage;
use atlas_core::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_core::state_transfer::Checkpoint;
use crate::backlog::{ConsensusBacklog, ConsensusBackLogHandle};
use crate::serialize::{PersistableStatefulOrderProtocol, PSDecLog, PSMessage, PSProof, PSView};

pub mod serialize;
pub mod backlog;
mod persistency;


/// The general type for a callback.
/// Callbacks are optional and can be used when you want to
/// execute a function when the logger stops finishes the computation
pub type CallbackType = Box<dyn FnOnce(Result<ResponseMsg>) + Send>;

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
pub struct PersistentLog<D: SharedData, PS: PersistableStatefulOrderProtocol>
{
    persistency_mode: PersistentLogMode<D>,

    // A handle for the persistent log workers (each with his own thread)
    worker_handle: Arc<PersistentLogWorkerHandle<D, PS>>,

    ///The persistent KV-DB to be used
    db: KVDB,
}

/// A handle for all of the persistent workers.
/// Handles task distribution and load balancing across the
/// workers
pub struct PersistentLogWorkerHandle<D: SharedData, PS: PersistableStatefulOrderProtocol> {
    round_robin_counter: AtomicUsize,
    tx: Vec<PersistentLogWriteStub<D, PS>>,
}

///A stub that is only useful for writing to the persistent log
#[derive(Clone)]
struct PersistentLogWriteStub<D: SharedData, PS: PersistableStatefulOrderProtocol> {
    tx: ChannelSyncTx<PWMessage<D, PS>>,
}

/// The type of the installed state information
pub type InstallState<D: SharedData, PS: PersistableStatefulOrderProtocol> = (
    //The view sequence number
    SeqNo,
    // The state that we want to persist
    Arc<ReadOnly<Checkpoint<D::State>>>,
    //The decision log that comes after that state
    PSDecLog<PS>,
);

/// Work messages for the
pub(crate) enum PWMessage<D: SharedData, PS: PersistableStatefulOrderProtocol> {
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
    Proof(PS::ProofMetadata, Vec<StoredMessage<PSMessage<PS>>>),

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

