use crate::ordering::tbo_queue::btree_tbo_queue::TboQueue;
use crate::ordering::tbo_queue::test::Message;

#[test]
fn test_tbo_queue_can_not_pop_until_adv() {
    super::test_tbo_queue_can_not_pop_until_adv::<TboQueue<Message>>();
}

#[test]
fn test_tbo_queue_can_pop_after_adv() {
    super::test_tbo_queue_can_pop_after_adv::<TboQueue<Message>>();
}

#[test]
fn test_adv_skips_old_messages() {
    super::test_adv_skips_old_messages::<TboQueue<Message>>();
}

#[test]
fn test_clear_queue() {
    super::test_clear_queue::<TboQueue<Message>>();
}

#[test]
fn test_reject_old_messages() {
    super::test_reject_old_messages::<TboQueue<Message>>();
}

#[test]
fn test_install_seq_discards_old_messages() {
    super::test_install_seq_discards_old_messages::<TboQueue<Message>>();
}

#[test]
fn test_install_seq_with_current_seq_no_change() {
    super::test_install_seq_with_current_seq_no_change::<TboQueue<Message>>();
}

#[test]
fn test_is_empty_consistent_with_peek() {
    super::test_is_empty_consistent_with_peek::<TboQueue<Message>>();
}

#[test]
fn test_is_empty_consistent_with_pop() {
    super::test_is_empty_consistent_with_pop::<TboQueue<Message>>();
}

#[test]
fn test_fifo_ordering_same_sequence() {
    super::test_fifo_ordering_same_sequence::<TboQueue<Message>>();
}

#[test]
fn test_out_of_order_insertion() {
    super::test_out_of_order_insertion::<TboQueue<Message>>();
}

#[test]
fn test_peek_does_not_remove() {
    super::test_peek_does_not_remove::<TboQueue<Message>>();
}

#[test]
fn test_multiple_advance_seq() {
    super::test_multiple_advance_seq::<TboQueue<Message>>();
}

#[test]
fn test_clear_preserves_sequence_number() {
    super::test_clear_preserves_sequence_number::<TboQueue<Message>>();
}

#[test]
fn test_interleaved_push_pop() {
    super::test_interleaved_push_pop::<TboQueue<Message>>();
}

#[test]
fn test_multiple_pops_same_sequence() {
    super::test_multiple_pops_same_sequence::<TboQueue<Message>>();
}

#[test]
fn test_peek_empty_after_advance() {
    super::test_peek_empty_after_advance::<TboQueue<Message>>();
}

#[test]
fn test_install_seq_after_consumption() {
    super::test_install_seq_after_consumption::<TboQueue<Message>>();
}

#[test]
fn test_push_same_sequence_multiple_times() {
    super::test_push_same_sequence_multiple_times::<TboQueue<Message>>();
}

#[test]
fn test_gaps_in_sequence_numbers() {
    super::test_gaps_in_sequence_numbers::<TboQueue<Message>>();
}

#[test]
fn test_clear_empty_queue() {
    super::test_clear_empty_queue::<TboQueue<Message>>();
}

#[test]
fn test_push_after_clear() {
    super::test_push_after_clear::<TboQueue<Message>>();
}

#[test]
fn test_install_seq_far_future() {
    super::test_install_seq_far_future::<TboQueue<Message>>();
}
