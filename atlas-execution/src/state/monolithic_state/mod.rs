#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::SeqNo;

pub struct InstallStateMessage<S> where S: MonolithicState {
    state: S,
}

pub struct AppStateMessage<S> where S: MonolithicState {
    seq: SeqNo,
    state: S,
}

/// The type abstraction for a monolithic state (only needs to be serializable, in reality)
#[cfg(feature = "serialize_serde")]
pub trait MonolithicState: for<'a> Deserialize<'a> + Serialize + Send + Clone {}

#[cfg(feature = "serialize_capnp")]
pub trait MonolithicState: Send + Clone {}


impl<S> AppStateMessage<S> where S: MonolithicState {
    pub fn new(seq: SeqNo, state: S) -> Self {
        AppStateMessage {
            seq,
            state,
        }
    }

    pub fn seq(&self) -> SeqNo {
        self.seq
    }

    pub fn state(&self) -> &S {
        &self.state
    }
}

impl<S> InstallStateMessage<S> where S: MonolithicState {
    pub fn new(state: S) -> Self {
        InstallStateMessage {
            state,
        }
    }

    pub fn state(&self) -> &S {
        &self.state
    }

    pub fn into_state(self) -> S {
        self.state
    }
}