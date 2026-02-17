mod vec_single_tbo_queue;
mod test;

use thiserror::Error;
use crate::ordering::{InvalidSeqNo, Orderable, SeqNo};

/// A TBO implementation similar to [`TTboQueue`] but only allows for a single message per sequence number.
/// Maintains a total order of messages based on their sequence numbers, allows for out-of-order insertion of messages.
/// Messages will only be popped when their sequence number matches the current sequence number of the queue.
pub trait TSingleTboQueue<M>: Orderable + Default {

    /// Push a new message into the TBO queue. Will only be popped
    /// when its sequence number matches the current sequence number of the queue.
    /// Returns an error if the message has a sequence number
    /// that is too less than the current sequence number of the queue, or if there is
    /// already a message with the same sequence number in the queue.
    fn push(&mut self, message: M) -> Result<(), PushItemResult>
    where
        M: Orderable;

    /// Returns true if there is no message available at the current sequence number.
    fn is_empty(&self) -> bool;

    /// Peeks at the next message in the TBO queue for the current sequence number, if it exists.
    fn peek(&self) -> Option<&M>;

    /// Pops the next message from the TBO queue for the current sequence number, if it exists.
    fn pop(&mut self) -> Option<M>;

    /// Advances the current sequence number of the queue,
    fn advance_seq(&mut self);

    /// Installs a new sequence number for the queue, which may be used to skip over
    /// a range of sequence numbers. Any messages with sequence numbers that are now too old to be popped
    /// will be discarded.
    fn install_seq(&mut self, seq_no: SeqNo);

    /// Clears all messages from the queue, does not change the current sequence number.
    fn clear(&mut self);

}

#[derive(Error, Debug)]
pub enum PushItemResult {
    #[error("Sequence number is already occupied: {0:?}")]
    AlreadyOccupied(SeqNo),
    #[error("Invalid sequence number: {0:?}")]
    InvalidSeq(#[from] InvalidSeqNo)
}