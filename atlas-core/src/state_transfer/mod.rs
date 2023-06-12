use std::sync::Arc;
use atlas_execution::serialize::SharedData;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::StoredMessage;
use atlas_communication::Node;
use crate::messages::{Protocol, StateTransfer};
use crate::ordering_protocol::{ExecutionResult, OrderingProtocol, OrderingProtocolArgs, View};
use crate::serialize::{NetworkView, OrderingProtocolMessage, ServiceMsg, StatefulOrderProtocolMessage, StateTransferMessage};
use crate::timeouts::{RqTimeout, Timeouts};
#[cfg(feature = "serialize_serde")]
use serde::{Serialize, Deserialize};
use atlas_common::crypto::hash::Digest;
use atlas_execution::ExecutorHandle;
use crate::persistent_log::{StatefulOrderingProtocolLog, StateTransferPersistentLog};
use crate::request_pre_processing::BatchOutput;


/// Represents a local checkpoint.
///
/// Contains the last application state, as well as the sequence number
/// which decided the last batch of requests executed before the checkpoint.
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub struct Checkpoint<S> {
    seq: SeqNo,
    app_state: S,
    digest: Digest,
}

impl<S> Orderable for Checkpoint<S> {
    /// Returns the sequence number of the batch of client requests
    /// decided before the local checkpoint.
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl<S> Checkpoint<S> {
    pub fn new(seq: SeqNo, app_state: S, digest: Digest) -> Arc<ReadOnly<Self>> {
        Arc::new(ReadOnly::new(Self {
            seq,
            app_state,
            digest,
        }))
    }

    /// The last sequence no represented in this checkpoint
    pub fn last_seq(&self) -> &SeqNo {
        &self.seq
    }

    /// Returns a reference to the state of the application before
    /// the local checkpoint.
    pub fn state(&self) -> &S {
        &self.app_state
    }

    pub fn digest(&self) -> &Digest { &self.digest }

    /// Returns the inner values within this local checkpoint.
    pub fn into_inner(self) -> (SeqNo, S, Digest) {
        (self.seq, self.app_state, self.digest)
    }
}

/// The result of processing a message in the state transfer protocol
pub enum STResult<D: SharedData> {
    RunCst,
    CstNotNeeded,
    CstRunning,
    CstFinished(D::State, Vec<D::Request>),
}

pub enum STTimeoutResult {
    RunCst,
    CstNotNeeded,
}

pub type CstM<M: StateTransferMessage> = <M as StateTransferMessage>::StateTransferMessage;

/// A trait for the implementation of the state transfer protocol
pub trait StateTransferProtocol<D, OP, NT, PL> where
    D: SharedData + 'static,
    OP: StatefulOrderProtocol<D, NT, PL> + 'static {
    /// The type which implements StateTransferMessage, to be implemented by the developer
    type Serialization: StateTransferMessage + 'static;

    /// The configuration type the protocol wants to accept
    type Config;

    /// Initialize the state transferring protocol with the given configuration, timeouts and communication layer
    fn initialize(config: Self::Config, timeouts: Timeouts, node: Arc<NT>, log: PL) -> Result<Self>
        where Self: Sized;

    /// Request the latest state from the rest of replicas
    fn request_latest_state(&mut self,
                            order_protocol: &mut OP) -> Result<()>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Handle a state transfer protocol message that was received while executing the ordering protocol
    fn handle_off_ctx_message(&mut self,
                              order_protocol: &mut OP,
                              message: StoredMessage<StateTransfer<CstM<Self::Serialization>>>)
                              -> Result<()>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Process a state transfer protocol message, received from other replicas
    /// We also provide a mutable reference to the stateful ordering protocol, so the 
    /// state can be installed (if that's the case)
    fn process_message(&mut self,
                       order_protocol: &mut OP,
                       message: StoredMessage<StateTransfer<CstM<Self::Serialization>>>)
                       -> Result<STResult<D>>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Handle the replica wanting to request a state from the application
    /// The state transfer protocol then sees if the conditions are met to receive it
    /// (We could still be waiting for a previous checkpoint, for example)
    fn handle_app_state_requested(&mut self,
                                  seq: SeqNo) -> Result<ExecutionResult>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Handle having received a state from the application
    /// you should also notify the ordering protocol that the state has been received
    /// and processed, so he is now safe to delete the state (Maybe this should be handled by the replica?)
    fn handle_state_received_from_app(&mut self,
                                      order_protocol: &mut OP,
                                      state: Arc<ReadOnly<Checkpoint<D::State>>>) -> Result<()>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;

    /// Handle a timeout being received from the timeout layer
    fn handle_timeout(&mut self, order_protocol: &mut OP, timeout: Vec<RqTimeout>) -> Result<STTimeoutResult>
        where NT: Node<ServiceMsg<D, OP::Serialization, Self::Serialization>>;
}

pub type DecLog<OP> = <OP as StatefulOrderProtocolMessage>::DecLog;

/// An order protocol that uses the state transfer protocol to manage its state.
pub trait StatefulOrderProtocol<D: SharedData + 'static, NT, PL>: OrderingProtocol<D, NT, PL> {
    /// The serialization abstraction for
    type StateSerialization: StatefulOrderProtocolMessage + 'static;

    fn initialize_with_initial_state(config: Self::Config, args: OrderingProtocolArgs<D, NT, PL>,
                                     dec_log: DecLog<Self::StateSerialization>) -> Result<Self> where
        Self: Sized;

    /// Install a state received from other replicas in the system
    /// Should only alter the necessary things within its own state and
    /// then should return the state and a list of all requests that should
    /// then be executed by the application.
    fn install_state(&mut self, view_info: View<Self::Serialization>,
                     dec_log: DecLog<Self::StateSerialization>) -> Result<Vec<D::Request>>
        where PL: StatefulOrderingProtocolLog<Self::Serialization, Self::StateSerialization>;

    /// Snapshot the current log of the replica
    fn snapshot_log(&mut self) -> Result<(View<Self::Serialization>, DecLog<Self::StateSerialization>)>
        where PL: StatefulOrderingProtocolLog<Self::Serialization, Self::StateSerialization>;

    /// Notify the consensus protocol that we have a checkpoint at a given sequence number,
    /// meaning it can now be garbage collected
    fn checkpointed(&mut self, seq: SeqNo) -> Result<()>
        where PL: StatefulOrderingProtocolLog<Self::Serialization, Self::StateSerialization>;
}
