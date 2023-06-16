#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use atlas_common::crypto::hash::Digest;
use atlas_common::ordering::Orderable;

pub trait PartId: PartialEq + PartialOrd + Clone {

    fn content_description(&self) -> Digest;

}

/// The abstraction for a divisible state, to be used by the state transfer protocol
pub trait DivisibleStateDescriptor: Orderable + PartialEq + Clone + Send {

    type PartDescription: PartId;

    /// Get all the parts of the state
    fn parts(&self) -> &Vec<Self::PartDescription>;

    /// Compare two states
    fn compare_descriptors(&self, other: &Self) -> Vec<Self::PartDescription>;

}

pub trait DivisibleState {

    #[cfg(feature = "serialize_serde")]
    type StateDescriptor: DivisibleStateDescriptor + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type StateDescriptor: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type StatePart: PartId + for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type StatePart: PartId + Send + Clone;

    /// Get the description of the state at this moment
    fn get_descriptor(&self) -> &Self::StateDescriptor;

    /// Accept a number of parts into our current state
    fn accept_parts(&mut self, parts: Vec<Self::StatePart>);

}