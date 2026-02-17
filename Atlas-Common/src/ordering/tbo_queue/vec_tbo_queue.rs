use crate::ordering::tbo_queue::{SeqMessageEntry, TTboQueue};
use crate::ordering::{InvalidSeqNo, Orderable, SeqNo};
use either::Either;
use std::collections::VecDeque;

pub struct VTboQueue<M> {
    current_seq_no: SeqNo,
    message_queue: VecDeque<SeqMessageEntry<M>>,
}

impl<M> Default for VTboQueue<M> {
    fn default() -> Self {
        Self::new()
    }
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
                    let last_seq_no =
                        self.current_seq_no + SeqNo::from(self.message_queue.len() as u32);

                    let len = index - self.message_queue.len() + 1;

                    for i in 0..len {
                        self.message_queue.push_back(SeqMessageEntry(
                            last_seq_no + SeqNo::from(i as u32),
                            VecDeque::new(),
                        ));
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
            Either::Right(index) => self.message_queue.get(index),
        }
    }

    fn get_entry_for_seq_no_mut(&mut self, seq_no: &SeqNo) -> Option<&mut SeqMessageEntry<M>> {
        match seq_no.index(self.current_seq_no) {
            Either::Left(_) => unreachable!(),
            Either::Right(index) => self.message_queue.get_mut(index),
        }
    }
}

impl<M> Orderable for VTboQueue<M> {
    fn sequence_number(&self) -> SeqNo {
        self.current_seq_no
    }
}

impl<M> TTboQueue<M> for VTboQueue<M> {
    fn push(&mut self, message: M) -> Result<(), InvalidSeqNo>
    where
        M: Orderable,
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

    fn is_empty(&self) -> bool {
        self.get_entry_for_seq_no(&self.current_seq_no)
            .map(|e| e.1.is_empty())
            .unwrap_or(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ordering::tbo_queue::test::*;

    #[test]
    fn test_vec_can_not_pop_until_adv() {
        test_tbo_queue_can_not_pop_until_adv::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_can_pop_after_adv() {
        test_tbo_queue_can_pop_after_adv::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_adv_skips_old_messages() {
        test_adv_skips_old_messages::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_clear_queue() {
        test_clear_queue::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_reject_old_messages() {
        test_reject_old_messages::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_install_seq_discards_old_messages() {
        test_install_seq_discards_old_messages::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_install_seq_with_current_seq_no_change() {
        test_install_seq_with_current_seq_no_change::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_is_empty_consistent_with_peek() {
        test_is_empty_consistent_with_peek::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_is_empty_consistent_with_pop() {
        test_is_empty_consistent_with_pop::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_fifo_ordering_same_sequence() {
        test_fifo_ordering_same_sequence::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_out_of_order_insertion() {
        test_out_of_order_insertion::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_peek_does_not_remove() {
        test_peek_does_not_remove::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_multiple_advance_seq() {
        test_multiple_advance_seq::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_clear_preserves_sequence_number() {
        test_clear_preserves_sequence_number::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_interleaved_push_pop() {
        test_interleaved_push_pop::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_multiple_pops_same_sequence() {
        test_multiple_pops_same_sequence::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_peek_empty_after_advance() {
        test_peek_empty_after_advance::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_install_seq_after_consumption() {
        test_install_seq_after_consumption::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_push_same_sequence_multiple_times() {
        test_push_same_sequence_multiple_times::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_gaps_in_sequence_numbers() {
        test_gaps_in_sequence_numbers::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_clear_empty_queue() {
        test_clear_empty_queue::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_push_after_clear() {
        test_push_after_clear::<VTboQueue<Message>>();
    }

    #[test]
    fn test_vec_install_seq_far_future() {
        test_install_seq_far_future::<VTboQueue<Message>>();
    }
}
