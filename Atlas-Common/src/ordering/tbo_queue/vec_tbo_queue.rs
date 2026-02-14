use crate::ordering::tbo_queue::{SeqMessageEntry, TTboQueue};
use crate::ordering::{InvalidSeqNo, Orderable, SeqNo};
use std::collections::VecDeque;
use either::Either;

pub struct VTboQueue<M> {
    current_seq_no: SeqNo,
    message_queue: VecDeque<SeqMessageEntry<M>>,
}

impl<M> VTboQueue<M> {

    pub fn new() -> Self {
        Self {
            current_seq_no: SeqNo::ZERO,
            message_queue: VecDeque::new(),
        }
    }

    fn get_or_insert_entry_for_seq_no(&mut self, seq_no: SeqNo) -> &mut SeqMessageEntry<M> {
        match seq_no.index(self.current_seq_no) {
            Either::Left(_) => {
                unreachable!()
            }
            Either::Right(index) => {
                if index >= self.message_queue.len() {
                    let last_seq_no = self.current_seq_no + SeqNo::from(self.message_queue.len() as u32);
                    
                    let len = index - self.message_queue.len() + 1;
                    
                    for i in 0..len {
                        self.message_queue.push_back(SeqMessageEntry(last_seq_no + SeqNo::from(i as u32), VecDeque::new()));
                    }
                }

                let entry = &mut self.message_queue[index];

                entry
            }
        }
    }

    fn get_entry_for_seq_no(&self, seq_no: &SeqNo) -> Option<&SeqMessageEntry<M>> {
        match seq_no.index(self.current_seq_no) {
            Either::Left(_) => unreachable!(),
            Either::Right(index) => {
                self.message_queue.get(index)
            }
        }
    }

    fn get_entry_for_seq_no_mut(&mut self, seq_no: &SeqNo) -> Option<&mut SeqMessageEntry<M>> {
        match seq_no.index(self.current_seq_no) {
            Either::Left(_) => unreachable!(),
            Either::Right(index) => {
                self.message_queue.get_mut(index)
            }
        }
    }

}

impl<M> Orderable for VTboQueue<M>
{
    fn sequence_number(&self) -> SeqNo {
        self.current_seq_no
    }
}

impl<M> TTboQueue<M> for VTboQueue<M> {
    fn push(&mut self, message: M) -> Result<(), InvalidSeqNo>
    where
        M: Orderable
    {
        match message.sequence_number().index(self.current_seq_no) {
            Either::Left(_) => return Err(InvalidSeqNo::Small),
            Either::Right(_) => {}
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
        self.get_entry_for_seq_no_mut(&seq_no)?.1.pop_front()
    }

    fn advance_seq(&mut self) {
        self.message_queue.pop_front();
        
        self.current_seq_no = self.current_seq_no.next();
    }

    fn install_seq(&mut self, seq_no: SeqNo) {
        
    }

    fn clear(&mut self) {
        self.message_queue.clear();
    }
}
