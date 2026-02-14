use crate::ordering::{InvalidSeqNo, Orderable, SeqNo};
use either::Either;
use std::collections::{BTreeMap, VecDeque};
use crate::ordering::tbo_queue::{SeqMessageEntry, TTboQueue};

/// A TBO (Total-Buffered-Ordering) queue is a data structure that maintains a total order of
/// messages based on their sequence numbers.
/// It allows for out-of-order insertion of messages
/// but ensures that messages are processed in the correct order when they are popped from the queue.
pub struct TboQueue<M> {
    current_seq_no: SeqNo,
    /// To avoid having to store empty entries in the queue,
    /// we use a BTreeMap to store the messages,
    /// where the key is the sequence number and the value is a queue of messages with that sequence number.
    message_queue: BTreeMap<SeqNo, SeqMessageEntry<M>>,
}

impl<M> Orderable for TboQueue<M>
where
    M: Orderable,
{
    fn sequence_number(&self) -> SeqNo {
        self.current_seq_no
    }
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

    fn get_or_insert_entry_for_seq_no(&mut self, seq_no: SeqNo) -> &mut SeqMessageEntry<M> {
        self.message_queue
            .entry(seq_no)
            .or_insert_with(|| SeqMessageEntry(seq_no, VecDeque::new()))
    }

    fn get_entry_for_seq_no(&self, seq_no: &SeqNo) -> Option<&SeqMessageEntry<M>> {
        self.message_queue.get(seq_no)
    }

    fn get_entry_for_seq_no_mut(&mut self, seq_no: &SeqNo) -> Option<&mut SeqMessageEntry<M>> {
        self.message_queue.get_mut(seq_no)
    }
}

impl<M> TTboQueue<M> for TboQueue<M>
where
    M: Orderable,
{
    fn push(&mut self, message: M) -> Result<(), InvalidSeqNo> {
        match message.sequence_number().index(self.current_seq_no) {
            Either::Right(_) => {}
            Either::Left(_) => return Err(InvalidSeqNo::Small),
        };

        self.get_or_insert_entry_for_seq_no(message.sequence_number())
            .1
            .push_back(message);

        Ok(())
    }

    fn peek(&self) -> Option<&M> {
        self.get_entry_for_seq_no(&self.current_seq_no)?.1.front()
    }

    fn pop(&mut self) -> Option<M> {
        let seq_no = self.current_seq_no;
        let entry = self.get_entry_for_seq_no_mut(&seq_no)?;

        entry.1.pop_front()
    }

    fn advance_seq(&mut self) {
        self.message_queue.remove(&self.current_seq_no);

        self.current_seq_no = self.current_seq_no.next();
    }

    fn install_seq(&mut self, seq_no: SeqNo) {
        if seq_no < self.current_seq_no {
            
        } else {
            
        }
    }

    fn clear(&mut self) { 
        self.message_queue.clear();
    }
}
