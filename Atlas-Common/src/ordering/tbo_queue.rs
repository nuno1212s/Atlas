use std::collections::{BTreeMap, VecDeque};
use crate::ordering::{Orderable, SeqNo};

/// A TBO (Total-Buffered-Ordering) queue is a data structure that maintains a total order of
/// messages based on their sequence numbers.
/// It allows for out-of-order insertion of messages
/// but ensures that messages are processed in the correct order when they are popped from the queue.
pub struct TboQueue<M> {
    current_seq_no: SeqNo,
    /// To avoid having to store empty entries in the queue,
    /// we use a BTreeMap to store the messages,
    /// where the key is the sequence number and the value is a queue of messages with that sequence number.
    message_queue: BTreeMap<SeqNo, SeqMessageEntry<M>>
}

impl<M> TboQueue<M>
where
    M: Orderable,
{
    pub fn new() -> Self {
        Self {
            current_seq_no: SeqNo::ZERO,
            message_queue: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, message: M) {
    }

    pub fn peek(&self) -> Option<&M> {
        todo!()
    }

    pub fn pop(&mut self) -> Option<M> {
        todo!()
    }
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