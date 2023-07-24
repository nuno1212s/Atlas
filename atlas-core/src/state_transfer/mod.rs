use std::sync::Arc;

#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::StoredMessage;
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_execution::serialize::ApplicationData;

use crate::messages::StateTransfer;
use crate::ordering_protocol::{ExecutionResult, OrderingProtocol};
use crate::persistent_log::StatefulOrderingProtocolLog;
use crate::serialize::{LogTransferMessage, NetworkView, OrderingProtocolMessage, StatefulOrderProtocolMessage, StateTransferMessage};
use crate::timeouts::RqTimeout;

pub mod monolithic_state;
pub mod divisible_state;
pub mod networking;

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

    pub fn new_simple(seq: SeqNo, app_state: S, digest: Digest) -> Self {
        Self {
            seq,
            app_state,
            digest,
        }
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
pub enum STResult {
    /// The message was processed successfully and
    /// we must run the state transfer protocol
    RunStateTransfer,
    /// The message was processed successfully and the ST protocol
    /// is not needed
    StateTransferNotNeeded(SeqNo),
    /// The message was processed successfully and the ST protocol
    /// is still running
    StateTransferRunning,
    /// The message was processed successfully and the ST protocol
    /// is running but there is already a partial state ready to
    /// be received by the executor
    StateTransferReady,
    /// The message was processed successfully and the ST protocol
    /// has finished
    StateTransferFinished(SeqNo),
}

/// The result of processing a message in the state transfer protocol
pub enum STTimeoutResult {
    RunCst,
    CstNotNeeded,
}

pub type CstM<M: StateTransferMessage> = <M as StateTransferMessage>::StateTransferMessage;

pub trait StateTransferProtocol<S, NT, PL> {
    /// The type which implements StateTransferMessage, to be implemented by the developer
    type Serialization: StateTransferMessage + 'static;

    /// Request the latest state from the rest of replicas
    fn request_latest_state<V>(&mut self, view: V) -> Result<()>
        where V: NetworkView;

    /// Handle a state transfer protocol message that was received while executing the ordering protocol
    fn handle_off_ctx_message<V>(&mut self,
                                 view: V,
                                 message: StoredMessage<StateTransfer<CstM<Self::Serialization>>>)
                                 -> Result<()>
        where V: NetworkView;

    /// Process a state transfer protocol message, received from other replicas
    /// We also provide a mutable reference to the stateful ordering protocol, so the
    /// state can be installed (if that's the case)
    fn process_message<V>(&mut self,
                          view: V,
                          message: StoredMessage<StateTransfer<CstM<Self::Serialization>>>)
                          -> Result<STResult>
        where V: NetworkView;

    /// Handle the replica wanting to request a state from the application
    /// The state transfer protocol then sees if the conditions are met to receive it
    /// (We could still be waiting for a previous checkpoint, for example)
    fn handle_app_state_requested<V>(&mut self,
                                     view: V,
                                     seq: SeqNo) -> Result<ExecutionResult>
        where V: NetworkView;

    /// Handle a timeout being received from the timeout layer
    fn handle_timeout<V>(&mut self,
                         view: V,
                         timeout: Vec<RqTimeout>) -> Result<STTimeoutResult>
        where V: NetworkView;
}