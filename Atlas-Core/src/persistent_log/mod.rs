use crate::ordering_protocol::networking::serialize::{
    OrderingProtocolMessage, PermissionedOrderingProtocolMessage,
};
use crate::ordering_protocol::{DecisionAD, DecisionMetadata, ShareableConsensusMessage, View};
use atlas_common::error::*;
use atlas_common::ordering::SeqNo;
use atlas_common::serialization_helper::SerMsg;

/// How should the data be written and response delivered?
/// If Sync is chosen the function will block on the call and return the result of the operation
/// If Async is chosen the function will not block and will return the response as a message to a channel
pub enum OperationMode {
    //When writing in async mode, you have the option of having the response delivered on a function
    //Of your choice
    //Note that this function will be executed on the persistent logging thread, so keep it short and
    //Be careful with race conditions.
    NonBlockingSync(Option<()>),
    BlockingSync,
}

pub trait PersistableStateTransferProtocol {}

/// The trait necessary for a persistent log protocol to be used as the persistent log layer
pub trait OrderingProtocolLog<RQ, OP>: Clone
where
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
{
    /// Write to the persistent log the latest committed sequence number
    fn write_committed_seq_no(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()>;

    /// Write a given message to the persistent log
    fn write_message(
        &self,
        write_mode: OperationMode,
        msg: ShareableConsensusMessage<RQ, OP>,
    ) -> Result<()>;

    /// Write the metadata for a given proof to the persistent log
    /// This in combination with the messages for that sequence number should form a valid proof
    fn write_decision_metadata(
        &self,
        write_mode: OperationMode,
        metadata: DecisionMetadata<RQ, OP>,
    ) -> Result<()>;

    /// Write the additional data for a given proof to the persistent log
    fn write_decision_additional_data(
        &self,
        write_mode: OperationMode,
        additional_data: DecisionAD<RQ, OP>,
    ) -> Result<()>;

    /// Invalidate all messages with sequence number equal to the given one
    fn write_invalidate(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()>;
}

/// The trait necessary for a permission logging protocol capable of simple
/// storage operations related to permissioned protocol messages
pub trait PermissionedOrderingProtocolLog<POP>
where
    POP: PermissionedOrderingProtocolMessage,
{
    /// Write a view info into the persistent log
    fn write_view_info(&self, write_mode: OperationMode, view: View<POP>) -> Result<()>;

    /// Read a view info from the persistent log
    fn read_view_info(&self) -> Result<Option<View<POP>>>;
}
