use crate::ordering::{InvalidSeqNo, Orderable, SeqNo};
use std::collections::VecDeque;

pub mod tbo_queue;
pub mod vec_tbo_queue;

/// A TBO (Total-Buffered-Ordering) queue is a data structure that maintains a total order of
/// messages based on their sequence numbers.
/// It allows for out-of-order insertion of messages
/// but ensures that messages are processed in the correct order when they are popped from the queue.
pub trait TTboQueue<M>: Orderable {

    /// Push a new message into the TBO queue. Will only be popped
    /// when its sequence number matches the current sequence number of the queue.
    /// Returns an error if the message has a sequence number
    /// that is too less than the current sequence number of the queue.
    fn push(&mut self, message: M) -> Result<(), InvalidSeqNo>
    where
        M: Orderable;

    /// Peeks at the next message in the TBO queue,
    /// if its sequence number matches the current sequence number of the queue.
    /// If there are multiple messages with the same sequence number,
    /// the one that was pushed first will be peeked at.
    fn peek(&self) -> Option<&M>;

    /// Pops the next message from the TBO queue,
    /// if its sequence number matches the current sequence number of the queue.
    /// If there are multiple messages with the same sequence number,
    /// they will be popped in the order they were pushed.
    fn pop(&mut self) -> Option<M>;

    /// Advances the current sequence number of the queue,
    /// allowing messages with the next sequence number to be popped.
    ///
    /// Discards any messages with sequence numbers that are now too old to be popped.
    fn advance_seq(&mut self);

    /// Installs a new sequence number for the queue, which may be used to skip over
    /// a range of sequence numbers. Any messages with sequence numbers that are now too old to be popped
    /// will be discarded.
    fn install_seq(&mut self, seq_no: SeqNo);

    /// Clears all messages from the queue, does not change the current sequence number.
    fn clear(&mut self);
}

struct SeqMessageEntry<M>(SeqNo, VecDeque<M>);

impl<M> Orderable for SeqMessageEntry<M> {
    fn sequence_number(&self) -> SeqNo {
        self.0
    }
}

impl<M> PartialEq<Self> for SeqMessageEntry<M> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<M> Eq for SeqMessageEntry<M> {}

impl<M> PartialOrd for SeqMessageEntry<M> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.0.cmp(&other.0))
    }
}

impl<M> Ord for SeqMessageEntry<M> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

#[cfg(test)]
mod perf_tests {

}

