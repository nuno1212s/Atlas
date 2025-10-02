use atlas_common::crypto::hash::Digest;
use atlas_common::error::*;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;

/// Messages to be sent from the state transfer module to the
/// executor module
pub enum InstallStateMessage<S>
where
    S: DivisibleState,
{
    /// We have received the descriptor of the state
    StateDescriptor(S::StateDescriptor),
    /// We have received a part of the state
    StatePart(MaybeVec<S::StatePart>),
    /// We can go back to polling the regular channel for new messages, as we are done installing state
    Done,
}

/// Messages to be sent by the executor for the state transfer module, notifying of a given
/// checkpoint being made
pub enum AppState<S>
where
    S: DivisibleState,
{
    StateDescriptor(S::StateDescriptor),
    StatePart(MaybeVec<S::StatePart>),
    Done,
}

/// The message that is sent when a checkpoint is done by the execution module
/// and a state must be returned for the state transfer protocol
pub struct AppStateMessage<S>
where
    S: DivisibleState,
{
    seq_no: SeqNo,
    state: AppState<S>,
}

/// The trait that represents the ID of a part
pub trait PartId: PartialEq + PartialOrd + Clone {
    fn content_description(&self) -> Digest;
}

/// The abstraction for a divisible state, to be used by the state transfer protocol
pub trait DivisibleStateDescriptor<S: DivisibleState>:
    Orderable + PartialEq + Clone + Send
{
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
pub trait DivisibleState: Sized + Send {
    type PartDescription: PartId + SerMsg;

    type StateDescriptor: DivisibleStateDescriptor<Self> + SerMsg;

    type StatePart: StatePart<Self> + SerMsg;

    /// Get the description of the state at this moment
    fn get_descriptor(&self) -> &Self::StateDescriptor;

    /// Accept a number of parts into our current state
    fn accept_parts(&mut self, parts: Vec<Self::StatePart>) -> Result<()>;

    /// Prepare a checkpoint of the state
    fn prepare_checkpoint(&mut self) -> Result<&Self::StateDescriptor>;

    /// Get the parts corresponding to the provided part descriptions
    fn get_parts(&self, parts: &[Self::PartDescription]) -> Result<Vec<Self::StatePart>>;
}

impl<S> AppStateMessage<S>
where
    S: DivisibleState,
{
    //Constructor
    pub fn new(seq_no: SeqNo, state_portion: AppState<S>) -> Self {
        AppStateMessage {
            seq_no,
            state: state_portion,
        }
    }

    pub fn into_state(self) -> (SeqNo, AppState<S>) {
        (self.seq_no, self.state)
    }
}

impl<S> Orderable for AppStateMessage<S>
where
    S: DivisibleState,
{
    fn sequence_number(&self) -> SeqNo {
        self.seq_no
    }
}
