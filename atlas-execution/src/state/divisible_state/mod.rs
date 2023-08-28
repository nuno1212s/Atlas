#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use atlas_common::error::*;
use atlas_common::crypto::hash::Digest;
use atlas_common::ordering::{Orderable, SeqNo};

pub enum InstallStateMessage<S> where S: DivisibleState {
    /// We have received a part of the state
    StatePart(Vec<S::StatePart>),
    /// We can go back to polling the regular channel for new messages, as we are done installing state
    Done
}

/// The message that is sent when a checkpoint is done by the execution module
/// and a state must be returned for the state transfer protocol
pub struct AppStateMessage<S> where S: DivisibleState {

    seq_no: SeqNo,
    state_descriptor: S::StateDescriptor,
    altered_parts: Vec<S::StatePart>,

}

/// The trait that represents the ID of a part
pub trait PartId: PartialEq + PartialOrd + Clone {

    fn content_description(&self) -> Digest;

}

/// The abstraction for a divisible state, to be used by the state transfer protocol
pub trait DivisibleStateDescriptor<S: DivisibleState>: Orderable + PartialEq + Clone + Send {

    /// Get all the parts of the state
    fn parts(&self) -> &Vec<S::PartDescription>;

    /// Compare two states
    fn compare_descriptors(&self, other: &Self) -> Vec<S::PartDescription>;

}

/// A part of the state
pub trait StatePart<S: DivisibleState> {

    fn descriptor(&self) -> S::PartDescription;

}

///
/// The trait that represents a divisible state, to be used by the state transfer protocol
///
pub trait DivisibleState: Sized {

    #[cfg(feature = "serialize_serde")]
    type PartDescription: PartId + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type PartDescription: PartId + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type StateDescriptor: DivisibleStateDescriptor<Self> + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type StateDescriptor: DivisibleStateDescriptor<Self> + Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type StatePart: StatePart<Self> + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type StatePart: StatePart<Self> + Send + Clone;

    /// Get the description of the state at this moment
    fn get_descriptor(&self) -> &Self::StateDescriptor;

    /// Accept a number of parts into our current state
    fn accept_parts(&mut self, parts: Vec<Self::StatePart>) -> Result<()>;

    /// Prepare a checkpoint of the state
    fn prepare_checkpoint(&mut self) -> Result<&Self::StateDescriptor>;

    /// Get the parts corresponding to the provided part descriptions
    fn get_parts(&self, parts: &Vec<Self::PartDescription>) -> Result<Vec<Self::StatePart>>;

}

impl<S> AppStateMessage<S> where S: DivisibleState {

    //Constructor
    pub fn new(seq_no: SeqNo, state_descriptor: S::StateDescriptor, altered_parts: Vec<S::StatePart>) -> Self {
        AppStateMessage {
            seq_no,
            state_descriptor,
            altered_parts,
        }
    }

    pub fn into_state(self) -> (S::StateDescriptor, Vec<S::StatePart>) {
        (self.state_descriptor, self.altered_parts)
    }

}

impl<S> Orderable for AppStateMessage<S> where S: DivisibleState {
    fn sequence_number(&self) -> SeqNo {
        self.seq_no
    }
}