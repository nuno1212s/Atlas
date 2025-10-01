use std::sync::Arc;

use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_core::persistent_log::OperationMode;
use atlas_smr_application::state::divisible_state::DivisibleState;
use atlas_smr_application::state::monolithic_state::MonolithicState;

use crate::state_transfer::Checkpoint;

///
/// The trait necessary for a logging protocol capable of handling monolithic states.
///
pub trait MonolithicStateLog<S>: Send
where
    S: MonolithicState,
{
    /// Read the local checkpoint from the persistent log
    fn read_checkpoint(&self) -> Result<Option<Checkpoint<S>>>;

    /// Write a checkpoint to the persistent log
    fn write_checkpoint(
        &self,
        write_mode: OperationMode,
        checkpoint: Arc<ReadOnly<Checkpoint<S>>>,
    ) -> Result<()>;
}

///
/// The trait necessary for a logging protocol capable of handling divisible states.
///
pub trait DivisibleStateLog<S>: Send
where
    S: DivisibleState,
{
    /// Read the descriptor of the local state
    fn read_local_descriptor(&self) -> Result<Option<S::StateDescriptor>>;

    /// Read a part from the local state log
    fn read_local_part(&self, part: S::PartDescription) -> Result<Option<S::StatePart>>;

    /// Write the descriptor of a state
    fn write_descriptor(
        &self,
        write_mode: OperationMode,
        checkpoint: S::StateDescriptor,
    ) -> Result<()>;

    /// Write a given set of parts to the log
    fn write_parts(
        &self,
        write_mode: OperationMode,
        parts: Vec<Arc<ReadOnly<S::StatePart>>>,
    ) -> Result<()>;

    /// Write a given set of parts and the descriptor of the state
    fn write_parts_and_descriptor(
        &self,
        write_mode: OperationMode,
        descriptor: S::StateDescriptor,
        parts: Vec<Arc<ReadOnly<S::StatePart>>>,
    ) -> Result<()>;

    /// Delete a given part from the log
    fn delete_part(&self, write_mode: OperationMode, part: S::PartDescription) -> Result<()>;
}
