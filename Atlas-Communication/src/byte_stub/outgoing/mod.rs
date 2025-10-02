use crate::byte_stub::outgoing::loopback::LoopbackOutgoingStub;

use crate::serialization::Serializable;

pub(super) mod loopback;

/// The outgoing stub for a peer.
///
/// Used to send messages to a peer.
/// If we were just doing binary messages, this would just be an impl [ByteNetworkStub].
/// Since we also want to handle loopback efficiently (without going through the binary stack, which
/// requires deserializing the message without need since we already have the parsed message and the digest/signature).
pub enum PeerOutgoingConnection<CN, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    OutgoingStub(CN),
    LoopbackStub(LoopbackOutgoingStub<R, O, S, A>),
}

impl<CN, R, O, S, A> Clone for PeerOutgoingConnection<CN, R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
    CN: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::OutgoingStub(cn) => Self::OutgoingStub(cn.clone()),
            Self::LoopbackStub(loop_back) => Self::LoopbackStub(loop_back.clone()),
        }
    }
}
