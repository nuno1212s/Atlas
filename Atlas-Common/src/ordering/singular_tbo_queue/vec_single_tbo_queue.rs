use crate::ordering::singular_tbo_queue::{PushItemResult, TSingleTboQueue};
use crate::ordering::{Orderable, SeqNo};
use either::Either;
use std::collections::VecDeque;

pub struct VSingleTBOQueue<M> {
    current_seq_no: SeqNo,
    message_queue: VecDeque<Option<M>>,
}

impl<M> Orderable for VSingleTBOQueue<M> {
    fn sequence_number(&self) -> SeqNo {
        self.current_seq_no
    }
}

impl<M> Default for VSingleTBOQueue<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> VSingleTBOQueue<M> {
    pub fn new() -> Self {
        Self {
            current_seq_no: SeqNo::ZERO,
            message_queue: VecDeque::new(),
        }
    }

    fn get_or_insert_entry_for_seq_no(&mut self, seq_no: SeqNo) -> &mut Option<M> {
        match seq_no.index(self.current_seq_no) {
            Either::Left(_) => {
                unreachable!()
            }
            Either::Right(index) => {
                if index >= self.message_queue.len() {
                    let len = index - self.message_queue.len() + 1;

                    for _ in 0..len {
                        self.message_queue.push_back(None);
                    }
                }

                let entry = &mut self.message_queue[index];

                entry
            }
        }
    }

    fn get_entry_for_seq_no(&self, seq_no: SeqNo) -> Option<&M> {
        match seq_no.index(self.current_seq_no) {
            Either::Left(_) => unreachable!(),
            Either::Right(index) => match self.message_queue.get(index) {
                None => None,
                Some(opt) => opt.as_ref(),
            },
        }
    }
}

impl<M> TSingleTboQueue<M> for VSingleTBOQueue<M> {
    fn push(&mut self, message: M) -> Result<(), PushItemResult>
    where
        M: Orderable,
    {
        let entry = self.get_or_insert_entry_for_seq_no(message.sequence_number());
        match entry {
            None => {
                *entry = Some(message);
                Ok(())
            }
            Some(_) => Err(PushItemResult::AlreadyOccupied(message.sequence_number())),
        }
    }

    fn is_empty(&self) -> bool {
        self.get_entry_for_seq_no(self.current_seq_no).is_none()
    }

    fn peek(&self) -> Option<&M> {
        self.get_entry_for_seq_no(self.current_seq_no)
    }

    fn pop(&mut self) -> Option<M> {
        let message = self.get_or_insert_entry_for_seq_no(self.current_seq_no);
        
        match message {
            None => None,
            Some(_) => message.take(),
        }
    }

    fn advance_seq(&mut self) {
        self.message_queue.pop_front();
        
        self.current_seq_no = self.current_seq_no.next();
    }

    fn install_seq(&mut self, seq_no: SeqNo) {
        match seq_no.index(self.current_seq_no) {
            Either::Left(_) => {
                self.current_seq_no = seq_no;
            }
            Either::Right(right) => {
                // we want to delete all entries with seq no < seq_no, which are the first `right` entries in the queue
                let to_delete = std::cmp::min(right, self.message_queue.len());

                for _ in 0..to_delete {
                    self.message_queue.pop_front();
                }

                self.current_seq_no = seq_no;
            }
        }
    }

    fn clear(&mut self) {
        self.message_queue.clear();
    }
}
