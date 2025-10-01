use crate::byte_stub::incoming::PeerIncomingConnection;
use crate::byte_stub::outgoing::PeerOutgoingConnection;
use crate::serialization::Serializable;
use getset::Getters;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The connection manager for a given peer
///
/// This struct implements the [NodeStubController] trait, which means it is responsible for
/// generating and managing stubs for connected peers. These stubs will then be used in the byte layer to
/// propagate messages upwards
#[derive(Getters)]
pub struct PeerConnection<CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    #[get = "pub"]
    pub(super) authenticated: Arc<AtomicBool>,
    #[get = "pub"]
    pub(super) incoming_connection: PeerIncomingConnection<R, O, S, A, L>,
    #[get = "pub"]
    pub(super) outgoing_connection: PeerOutgoingConnection<CN, R, O, S, A>,
}

impl<CN, R, O, S, A, L> PeerConnection<CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub fn is_authenticated(&self) -> bool {
        self.authenticated
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl<CN, R, O, S, A, L> Clone for PeerConnection<CN, R, O, S, A, L>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    L: Clone,
    CN: Clone,
{
    fn clone(&self) -> Self {
        Self {
            authenticated: self.authenticated.clone(),
            incoming_connection: self.incoming_connection.clone(),
            outgoing_connection: self.outgoing_connection.clone(),
        }
    }
}
