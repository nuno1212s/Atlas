use atlas_common::node_id::NodeId;
use std::io;
use std::sync::Arc;

use crate::byte_stub::connections::NetworkConnectionController;
use crate::lookup_table::MessageModule;
use crate::message::WireMessage;
use crate::reconfiguration::NetworkInformationProvider;
use atlas_common::channel::{SendError, TrySendError};
use thiserror::Error;

pub mod connections;
pub mod incoming;
pub(crate) mod outgoing;
pub(crate) mod peer_conn;
pub mod peer_conn_manager;
pub(crate) mod stub_endpoint;
#[cfg(test)]
mod test;

pub(crate) const MODULES: usize = enum_map::enum_len::<MessageModule>();

/// The byte network controller.
///
/// Meant to store type info and provide access to multiple things at the byte networking level
pub trait ByteNetworkController: Send + Sync + Clone {
    ///  The configuration type
    type Config: Send + 'static;

    /// The connection controller type, used to instruct the byte network layer
    /// to query the network level connections
    type ConnectionController: NetworkConnectionController + 'static;

    /// Get the reference to this controller's connection controller
    fn connection_controller(&self) -> &Arc<Self::ConnectionController>;
}

/// The generics are meant for:
/// NI: NetworkInformationProvider
/// NSC: The Node stub controller
/// BS: The byte network stub, which is connection oriented
/// IS: The incoming stub, which is connection oriented
pub trait ByteNetworkControllerInit<NI, NSC, BS, IS>: ByteNetworkController {
    type Error: std::error::Error;

    fn initialize_controller(
        network_info: Arc<NI>,
        config: Self::Config,
        stub_controllers: NSC,
    ) -> Result<Self, Self::Error>
    where
        Self: Sized,
        NI: NetworkInformationProvider,
        BS: ByteNetworkStub,
        IS: NodeIncomingStub,
        NSC: NodeStubController<BS, IS>;
}

/// The result of attempting to dispatch a message
/// If the message could not be dispatched, we should try again later
#[derive(Error, Debug)]
pub enum DispatchError {
    #[error("Could not dispatch the message, try again later. Message: {0:?}")]
    CouldNotDispatchTryLater(WireMessage),
    #[error("Failed due to error {0:?}")]
    InternalError(#[from] io::Error),
    #[error("Try send error {0:?}")]
    TrySendError(#[from] TrySendError),
    #[error("Send error {0:?}")]
    SendError(#[from] SendError),
}

impl From<DispatchError> for WireMessage {
    fn from(value: DispatchError) -> Self {
        match value {
            DispatchError::CouldNotDispatchTryLater(message) => message,
            _ => unreachable!("Cannot unbox an error as a message"),
        }
    }
}

impl ByteNetworkDispatchError for DispatchError {
    fn can_retry(&self) -> bool {
        matches!(self, DispatchError::CouldNotDispatchTryLater(_))
    }
}

/// The network stub for byte messages (after they have gone through the serialization process)
/// This is connection oriented, meaning each Stub which implements this trait should only
/// be referring to a single connection to a single peer
pub trait ByteNetworkStub: Send + Sync + Clone {
    type Error: std::error::Error + ByteNetworkDispatchError;

    // Dispatch a message to the peer this stub is responsible for.
    //
    fn dispatch_message(&self, message: WireMessage) -> Result<(), Self::Error> {
        self.dispatch_blocking(message)?;

        Ok(())
    }

    // Dispatch
    fn dispatch_blocking(&self, message: WireMessage) -> Result<(), Self::Error>;
}

pub trait ByteNetworkDispatchError: Into<WireMessage> {
    fn can_retry(&self) -> bool;
}

/// The stub controller, responsible for generating and managing stubs for connected peers.
///
/// See why we chose a model where we orient ourselves around the peer instead of the message
/// in [NodeIncomingStub]. It's basically revolving around maintaining context, reducing lookups on connection maps and non static look up tables (which would have to be protected with locks since they
/// would have to be modified on the fly).
///
/// This is a trait (and not directly the [PeerConnectionManager] which implements this trait and its behaviours)
/// because we want to obscure the generic information found at this module's level (R, O, S, A) from the byte network
/// level, such that the byte network only has to deal with bytes and a small amount of generics.
pub trait NodeStubController<BS, IS>: Send + Sync + Clone {
    type Error: std::error::Error + Send;

    // Check if a given node has a stub registered.
    fn has_stub_for(&self, node: &NodeId) -> bool;

    /// Generate a stub for a given node connection.
    /// Accepts the node id, the type of the node and the output stub that we can use to send byte messages (already serialized)
    ///
    /// Returns an implementation of [NodeIncomingStub] that we can use to handle messages we have received from that node.
    /// By handle we mean deserialize, verify and pass on to the correct stub
    fn generate_stub_for(&self, node: NodeId, byte_stub: BS) -> Result<IS, Self::Error>
    where
        BS: ByteNetworkStub,
        IS: NodeIncomingStub;

    // Get the stub that is responsible for handling messages from a given node.
    fn get_stub_for(&self, node: &NodeId) -> Option<IS>
    where
        IS: NodeIncomingStub;

    // Shutdown the active stubs for a given node (effectively closing the connection)
    fn shutdown_stubs_for(&self, node: &NodeId) -> Result<(), Self::Error>;
}

/// The stub that is responsible for handling messages from a given node.
/// This stub will be produced by [NodeStubController::get_or_generate_stub_for]
/// and will then be responsible for handling all messages we received from that given
/// node.
///
/// This is architectured in this way to facilitate connection oriented handling instead of message
/// oriented handling, which involves much more concurrency and therefore much less performance.
pub trait NodeIncomingStub: Send + Sync + Clone {
    type Error: std::error::Error;

    /// Handle a binary message being received, sent from the peer this stub is responsible for.
    /// This should trigger all necessary actions to handle the message. (Deserialization, signature verification, etc.)
    fn handle_message<NI>(
        &self,
        network_info: &Arc<NI>,
        message: WireMessage,
    ) -> Result<(), Self::Error>
    where
        NI: NetworkInformationProvider + 'static;
}

pub(crate) fn from_arr<T, const N: usize>(v: Vec<T>) -> Result<[T; N], ArrayCreationError> {
    v.try_into()
        .map_err(|v: Vec<T>| ArrayCreationError::InvalidLength(v.len(), N))
}

#[derive(Error, Debug)]
pub(crate) enum ArrayCreationError {
    #[error("Invalid length: expected {1}, got {0}")]
    InvalidLength(usize, usize),
}

#[derive(Error, Debug)]
#[error("Blank error")]
pub struct BlankError;
