use crate::ordering::singular_tbo_queue::{PushItemResult, TSingleTboQueue};
use crate::ordering::{InvalidSeqNo, Orderable, SeqNo};

pub struct Message(SeqNo, u32);

impl Orderable for Message {
    fn sequence_number(&self) -> SeqNo {
        self.0
    }
}

fn prepare_scenario<Q>(queue: &mut Q, sequence_nos: usize)
where
    Q: TSingleTboQueue<Message>,
{
    for i in 0..sequence_nos {
        let message = Message(SeqNo::from(i as u32), i as u32);
        queue.push(message).unwrap();
    }
}

pub fn test_single_tbo_queue_can_not_pop_until_adv<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    prepare_scenario(&mut queue, 1);

    let pop = queue.pop();
    assert!(pop.is_some());

    let message = pop.unwrap();

    assert_eq!(SeqNo::ZERO, message.0);

    assert!(queue.pop().is_none());
    assert!(queue.is_empty());
}

pub fn test_single_tbo_queue_can_pop_after_adv<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    prepare_scenario(&mut queue, 2);

    let pop = queue.pop();
    assert!(pop.is_some());

    let message = pop.unwrap();

    assert_eq!(SeqNo::ZERO, message.0);

    assert!(queue.pop().is_none());

    queue.advance_seq();

    let pop = queue.pop();
    assert!(pop.is_some());

    let message = pop.unwrap();

    assert_eq!(SeqNo::ONE, message.0);

    assert!(queue.pop().is_none());
    assert!(queue.is_empty());
}

pub fn test_adv_skips_old_messages<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    let messages = vec![Message(SeqNo::ZERO, 0), Message(SeqNo::ONE, 0)];

    messages.into_iter().for_each(|m| {
        queue.push(m).unwrap();
    });

    let peek = queue.peek();
    assert!(peek.is_some());
    assert_eq!(SeqNo::ZERO, peek.map(Orderable::sequence_number).unwrap());

    queue.advance_seq();

    let peek = queue.peek();

    assert!(peek.is_some());
    assert_eq!(SeqNo::ONE, peek.map(Orderable::sequence_number).unwrap());
}

pub fn test_clear_queue<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    prepare_scenario(&mut queue, 10);

    queue.clear();

    for _ in 0..10 {
        assert!(queue.is_empty());
        queue.advance_seq();
    }
}

/// Test that messages with sequence numbers less than the current sequence number
/// cannot be inserted and return InvalidSeqNo::Small error
pub fn test_reject_old_messages<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push message at seq 0
    let msg0 = Message(SeqNo::ZERO, 0);
    assert!(queue.push(msg0).is_ok());

    // Advance to seq 1
    queue.advance_seq();

    // Try to push message with seq 0 (now old)
    let old_msg = Message(SeqNo::ZERO, 1);
    let result = queue.push(old_msg);

    assert!(result.is_err());
}

/// Test install_seq behavior: it should discard old messages
/// and set new current sequence number
pub fn test_install_seq_discards_old_messages<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push messages for seq 0, 1, 2, 3, 4
    prepare_scenario(&mut queue, 5);

    // Current seq is 0, should have messages 0,1,2,3,4
    assert_eq!(SeqNo::ZERO, queue.sequence_number());

    // Install new sequence number 3
    queue.install_seq(SeqNo::from(3u32));

    // Current sequence should now be 3
    assert_eq!(SeqNo::from(3u32), queue.sequence_number());

    // Messages 0, 1, 2 should be discarded
    // We should not be able to pop them
    // The first available message should be from seq 3
    let first = queue.peek();
    assert!(first.is_some());
    assert_eq!(SeqNo::from(3u32), first.unwrap().0);
}

/// Test that install_seq with same sequence number doesn't change state
pub fn test_install_seq_with_current_seq_no_change<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    prepare_scenario(&mut queue, 3);

    // Current seq is 0
    assert_eq!(SeqNo::ZERO, queue.sequence_number());

    // Install same sequence
    queue.install_seq(SeqNo::ZERO);

    // Should still be at 0 with same message
    assert_eq!(SeqNo::ZERO, queue.sequence_number());
    assert_eq!(SeqNo::ZERO, queue.peek().unwrap().0);
}

/// Test that is_empty is consistent with peek
pub fn test_is_empty_consistent_with_peek<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Empty queue
    assert!(queue.is_empty());
    assert!(queue.peek().is_none());

    // Add message at current seq
    queue.push(Message(SeqNo::ZERO, 0)).unwrap();
    assert!(!queue.is_empty());
    assert!(queue.peek().is_some());

    // Pop it
    queue.pop();
    assert!(queue.is_empty());
    assert!(queue.peek().is_none());

    // Add message at future seq (not current)
    queue.push(Message(SeqNo::from(5u32), 0)).unwrap();
    assert!(queue.is_empty()); // Still empty at current seq
    assert!(queue.peek().is_none());
}

/// Test that is_empty is consistent with pop
pub fn test_is_empty_consistent_with_pop<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Empty queue
    assert!(queue.is_empty());
    assert!(queue.pop().is_none());

    // Add message
    queue.push(Message(SeqNo::ZERO, 0)).unwrap();
    assert!(!queue.is_empty());

    // Pop should succeed
    let popped = queue.pop();
    assert!(popped.is_some());

    // Now should be empty
    assert!(queue.is_empty());
    assert!(queue.pop().is_none());
}

/// Test that peek doesn't remove the message
pub fn test_peek_does_not_remove<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    queue.push(Message(SeqNo::ZERO, 42)).unwrap();

    // Peek multiple times
    assert_eq!(42, queue.peek().unwrap().1);
    assert_eq!(42, queue.peek().unwrap().1);
    assert_eq!(42, queue.peek().unwrap().1);

    // Pop should still get the message
    assert_eq!(42, queue.pop().unwrap().1);

    // Now should be empty
    assert!(queue.peek().is_none());
}

/// Test multiple advance_seq calls
pub fn test_multiple_advance_seq<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push messages for seq 0-9
    for i in 0..10 {
        queue
            .push(Message(SeqNo::from(i as u32), i as u32))
            .unwrap();
    }

    // Pop and advance multiple times
    for i in 0..10 {
        assert_eq!(i as u32, queue.pop().unwrap().1);
        if i < 9 {
            queue.advance_seq();
        }
    }

    assert!(queue.is_empty());
}

/// Test that clearing preserves the current sequence number
pub fn test_clear_preserves_sequence_number<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    prepare_scenario(&mut queue, 5);

    // Advance a few times
    queue.advance_seq();
    queue.advance_seq();

    let seq_before = queue.sequence_number();

    // Clear the queue
    queue.clear();

    // Sequence number should be unchanged
    assert_eq!(seq_before, queue.sequence_number());

    // Queue should be empty
    assert!(queue.is_empty());

    // Should be able to push new messages for future sequences
    queue.push(Message(SeqNo::from(5u32), 0)).unwrap();
    assert!(queue.is_empty()); // Still empty at current seq
}

/// Test interleaving push and pop operations
pub fn test_interleaved_push_pop<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push seq 0
    queue.push(Message(SeqNo::ZERO, 0)).unwrap();

    // Pop it
    assert_eq!(0, queue.pop().unwrap().1);
    assert!(queue.is_empty());

    // Push seq 1 (but current is still 0)
    queue.push(Message(SeqNo::ONE, 1)).unwrap();
    assert!(queue.is_empty());

    // Advance
    queue.advance_seq();

    // Now seq 1 should be available
    assert_eq!(1, queue.pop().unwrap().1);
}

/// Test peek on empty current sequence after advance
pub fn test_peek_empty_after_advance<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    queue.push(Message(SeqNo::ZERO, 0)).unwrap();
    queue.pop(); // Remove the message

    // Push something at seq 2
    queue.push(Message(SeqNo::from(2u32), 0)).unwrap();

    // Advance to seq 1
    queue.advance_seq();

    // Peek should return None (seq 1 doesn't have messages)
    assert!(queue.peek().is_none());

    // Pop should return None
    assert!(queue.pop().is_none());
}

/// Test install_seq after some messages have been consumed
pub fn test_install_seq_after_consumption<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    prepare_scenario(&mut queue, 10);

    // Pop first 3 messages
    queue.pop();
    queue.advance_seq();
    queue.pop();
    queue.advance_seq();
    queue.pop();
    queue.advance_seq();

    // Now at seq 3
    assert_eq!(SeqNo::from(3u32), queue.sequence_number());

    // Install seq 7
    queue.install_seq(SeqNo::from(7u32));

    // Should be at seq 7 now
    assert_eq!(SeqNo::from(7u32), queue.sequence_number());

    // Should be able to pop message 7
    let msg = queue.pop().unwrap();
    assert_eq!(SeqNo::from(7u32), msg.0);
}

/// Test with gaps in sequence numbers
pub fn test_gaps_in_sequence_numbers<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push seq 0, 2, 4 (skipping 1, 3)
    queue.push(Message(SeqNo::ZERO, 0)).unwrap();
    queue.push(Message(SeqNo::from(2u32), 2)).unwrap();
    queue.push(Message(SeqNo::from(4u32), 4)).unwrap();

    // Pop seq 0
    assert_eq!(0, queue.pop().unwrap().1);
    queue.advance_seq();

    // seq 1 doesn't exist, is_empty should return true
    assert!(queue.is_empty());
    assert!(queue.peek().is_none());

    // Advance past the gap
    queue.advance_seq();

    // Now seq 2 should be available
    assert!(!queue.is_empty());
    assert_eq!(2, queue.pop().unwrap().1);
}

/// Test clear on empty queue
pub fn test_clear_empty_queue<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    assert!(queue.is_empty());
    queue.clear();
    assert!(queue.is_empty());

    // Should still be at seq 0
    assert_eq!(SeqNo::ZERO, queue.sequence_number());
}

/// Test that after clear, we can push new messages
pub fn test_push_after_clear<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    queue.push(Message(SeqNo::ZERO, 0)).unwrap();
    queue.push(Message(SeqNo::ONE, 1)).unwrap();

    queue.clear();

    // Can push again
    queue.push(Message(SeqNo::ZERO, 100)).unwrap();
    assert_eq!(100, queue.pop().unwrap().1);
}

/// Test install_seq to a future sequence
pub fn test_install_seq_far_future<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    queue.push(Message(SeqNo::ZERO, 0)).unwrap();

    // Jump far ahead
    let far_seq = SeqNo::from(100u32);
    queue.install_seq(far_seq);

    assert_eq!(far_seq, queue.sequence_number());
    assert!(queue.is_empty());

    // Can now push at far_seq
    queue.push(Message(far_seq, 99)).unwrap();
    assert_eq!(99, queue.pop().unwrap().1);
}

/// Test that pushing the same sequence number twice returns AlreadyOccupied error
/// This is specific to singular_tbo_queue since regular tbo_queue allows multiple messages per sequence
pub fn test_push_same_sequence_twice_fails<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push message at seq 0
    let msg1 = Message(SeqNo::ZERO, 1);
    assert!(queue.push(msg1).is_ok());

    // Try to push another message at seq 0
    let msg2 = Message(SeqNo::ZERO, 2);
    let result = queue.push(msg2);

    assert!(result.is_err());
    match result {
        Err(PushItemResult::AlreadyOccupied(seq)) => {
            assert_eq!(SeqNo::ZERO, seq);
        }
        _ => panic!("Expected AlreadyOccupied error"),
    }
}

/// Test that pushing a message after consuming the previous one at the same sequence works
pub fn test_push_same_sequence_after_pop<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push and pop message at seq 0
    queue.push(Message(SeqNo::ZERO, 1)).unwrap();
    let popped = queue.pop().unwrap();
    assert_eq!(1, popped.1);

    // Should be able to push another message at seq 0 now
    // (though it will be at the same position)
    queue.push(Message(SeqNo::ZERO, 2)).unwrap();

    // Pop it
    let popped = queue.pop().unwrap();
    assert_eq!(2, popped.1);
}

/// Test out-of-order insertion with single message per sequence
pub fn test_out_of_order_insertion<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push seq 2 first
    queue.push(Message(SeqNo::from(2u32), 0)).unwrap();
    assert!(queue.is_empty()); // Can't pop yet

    // Push seq 0
    queue.push(Message(SeqNo::ZERO, 0)).unwrap();
    assert!(!queue.is_empty()); // Now can pop

    // Pop seq 0
    let msg = queue.pop().unwrap();
    assert_eq!(SeqNo::ZERO, msg.0);

    queue.advance_seq();
    assert!(queue.is_empty()); // Still waiting for seq 1

    // Push seq 1
    queue.push(Message(SeqNo::ONE, 0)).unwrap();
    assert!(!queue.is_empty());

    let msg = queue.pop().unwrap();
    assert_eq!(SeqNo::ONE, msg.0);

    queue.advance_seq();
    assert!(!queue.is_empty()); // Now seq 2 is available

    let msg = queue.pop().unwrap();
    assert_eq!(SeqNo::from(2u32), msg.0);
}

/// Test that AlreadyOccupied error preserves the original message
pub fn test_already_occupied_preserves_original<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push message at seq 0
    queue.push(Message(SeqNo::ZERO, 42)).unwrap();

    // Try to push another message at seq 0
    queue.push(Message(SeqNo::ZERO, 99)).unwrap_err();

    // Original message should still be there
    assert_eq!(42, queue.pop().unwrap().1);
}

/// Test install_seq backward (to past sequence) discards nothing if already past
pub fn test_install_seq_backward_does_nothing<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    prepare_scenario(&mut queue, 5);

    // Advance to seq 2
    queue.advance_seq();
    queue.advance_seq();

    let current_seq = queue.sequence_number();

    assert!(matches!(
        queue.install_seq(SeqNo::ZERO),
        Err(InvalidSeqNo::Small)
    ));

    // Should still be at seq 2 and have messages 2, 3, 4
    assert_eq!(current_seq, queue.sequence_number());
    assert!(!queue.is_empty());
}

/// Test sequence of operations: push, peek, advance, push, pop
pub fn test_complex_sequence_operations<Q>()
where
    Q: TSingleTboQueue<Message>,
{
    let mut queue = Q::default();

    // Push seq 0
    queue.push(Message(SeqNo::ZERO, 10)).unwrap();
    assert_eq!(10, queue.peek().unwrap().1);

    // Advance to seq 1
    queue.advance_seq();

    // Push seq 2 (skip 1)
    queue.push(Message(SeqNo::from(2u32), 20)).unwrap();
    assert!(queue.is_empty()); // seq 1 doesn't exist

    // Advance to seq 2
    queue.advance_seq();

    // Now seq 2 should be available
    assert_eq!(20, queue.pop().unwrap().1);
}
