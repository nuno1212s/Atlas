use crate::ordering::singular_tbo_queue::{PushItemResult, TSingleTboQueue};
use crate::ordering::{InvalidSeqNo, Orderable, SeqNo};
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
        if message.sequence_number() < self.current_seq_no {
            return Err(PushItemResult::InvalidSeq(InvalidSeqNo::Small));
        }

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

    fn install_seq(&mut self, seq_no: SeqNo) -> Result<(), InvalidSeqNo> {
        match seq_no.index(self.current_seq_no) {
            Either::Left(_) => Err(InvalidSeqNo::Small),
            Either::Right(right) => {
                // we want to delete all entries with seq no < seq_no, which are the first `right` entries in the queue
                let to_delete = std::cmp::min(right, self.message_queue.len());

                for _ in 0..to_delete {
                    self.message_queue.pop_front();
                }

                self.current_seq_no = seq_no;

                Ok(())
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
    use crate::ordering::singular_tbo_queue::test::*;

    #[test]
    fn test_vec_single_can_not_pop_until_adv() {
        test_single_tbo_queue_can_not_pop_until_adv::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_can_pop_after_adv() {
        test_single_tbo_queue_can_pop_after_adv::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_adv_skips_old_messages() {
        test_adv_skips_old_messages::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_clear_queue() {
        test_clear_queue::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_reject_old_messages() {
        test_reject_old_messages::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_install_seq_discards_old_messages() {
        test_install_seq_discards_old_messages::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_install_seq_with_current_seq_no_change() {
        test_install_seq_with_current_seq_no_change::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_is_empty_consistent_with_peek() {
        test_is_empty_consistent_with_peek::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_is_empty_consistent_with_pop() {
        test_is_empty_consistent_with_pop::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_peek_does_not_remove() {
        test_peek_does_not_remove::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_multiple_advance_seq() {
        test_multiple_advance_seq::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_clear_preserves_sequence_number() {
        test_clear_preserves_sequence_number::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_interleaved_push_pop() {
        test_interleaved_push_pop::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_peek_empty_after_advance() {
        test_peek_empty_after_advance::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_install_seq_after_consumption() {
        test_install_seq_after_consumption::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_gaps_in_sequence_numbers() {
        test_gaps_in_sequence_numbers::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_clear_empty_queue() {
        test_clear_empty_queue::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_push_after_clear() {
        test_push_after_clear::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_install_seq_far_future() {
        test_install_seq_far_future::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_push_same_sequence_twice_fails() {
        test_push_same_sequence_twice_fails::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_push_same_sequence_after_pop() {
        test_push_same_sequence_after_pop::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_out_of_order_insertion() {
        test_out_of_order_insertion::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_already_occupied_preserves_original() {
        test_already_occupied_preserves_original::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_install_seq_backward_does_nothing() {
        test_install_seq_backward_does_nothing::<VSingleTBOQueue<Message>>();
    }

    #[test]
    fn test_vec_single_complex_sequence_operations() {
        test_complex_sequence_operations::<VSingleTBOQueue<Message>>();
    }
}
