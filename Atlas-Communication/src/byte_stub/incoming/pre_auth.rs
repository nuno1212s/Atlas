//! Holds messages that arrive before their connection is authenticated, so the sender's first
//! window is delivered once it is, instead of being dropped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Per-connection bounds, so an unauthenticated peer cannot make us buffer without limit.
const MAX_HELD_MESSAGES: usize = 1024;
const MAX_HELD_BYTES: usize = 2 * 1024 * 1024;

pub(crate) enum Admission<M> {
    /// Process now.
    Deliver(M),
    /// Held until the connection authenticates.
    Held,
    /// Buffer full; the message comes back to be dropped.
    Full(M),
}

struct Held<M> {
    messages: Vec<M>,
    bytes: usize,
}

pub(crate) struct PreAuthGate<M> {
    authenticated: Arc<AtomicBool>,
    held: Mutex<Held<M>>,
}

impl<M> PreAuthGate<M> {
    pub(crate) fn new(authenticated: Arc<AtomicBool>) -> Self {
        Self {
            authenticated,
            held: Mutex::new(Held {
                messages: Vec::new(),
                bytes: 0,
            }),
        }
    }

    pub(crate) fn is_authenticated(&self) -> bool {
        self.authenticated.load(Ordering::Acquire)
    }

    /// `holdable`: the message must wait for authentication (anything but reconfiguration).
    pub(crate) fn admit(&self, message: M, holdable: bool, size: usize) -> Admission<M> {
        if !holdable || self.is_authenticated() {
            return Admission::Deliver(message);
        }

        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);

        // `authenticate` sets the flag under this lock, so this check cannot miss it.
        if self.is_authenticated() {
            return Admission::Deliver(message);
        }

        if held.messages.len() >= MAX_HELD_MESSAGES || held.bytes + size > MAX_HELD_BYTES {
            return Admission::Full(message);
        }

        held.bytes += size;
        held.messages.push(message);

        Admission::Held
    }

    /// Mark the connection authenticated and return what was held, in arrival order.
    pub(crate) fn authenticate(&self) -> Vec<M> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);

        self.authenticated.store(true, Ordering::Release);
        held.bytes = 0;

        std::mem::take(&mut held.messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::thread;

    fn gate(authenticated: bool) -> PreAuthGate<u32> {
        PreAuthGate::new(Arc::new(AtomicBool::new(authenticated)))
    }

    fn delivered(a: Admission<u32>) -> Option<u32> {
        match a {
            Admission::Deliver(m) => Some(m),
            _ => None,
        }
    }

    #[test]
    fn an_authenticated_connection_delivers_immediately() {
        assert_eq!(delivered(gate(true).admit(7, true, 10)), Some(7));
    }

    #[test]
    fn reconfiguration_traffic_is_never_held() {
        assert_eq!(delivered(gate(false).admit(7, false, 10)), Some(7));
    }

    #[test]
    fn messages_wait_for_authentication_and_replay_in_order() {
        let g = gate(false);

        for m in [1, 2, 3] {
            assert!(matches!(g.admit(m, true, 10), Admission::Held));
        }

        assert_eq!(g.authenticate(), vec![1, 2, 3]);
        assert!(g.is_authenticated());
        assert_eq!(delivered(g.admit(4, true, 10)), Some(4));
        assert!(g.authenticate().is_empty());
    }

    #[test]
    fn the_buffer_is_bounded_by_count_and_by_bytes() {
        let g = gate(false);

        for m in 0..MAX_HELD_MESSAGES as u32 {
            assert!(matches!(g.admit(m, true, 1), Admission::Held));
        }
        assert!(matches!(g.admit(0, true, 1), Admission::Full(0)));

        let g = gate(false);
        assert!(matches!(g.admit(1, true, MAX_HELD_BYTES), Admission::Held));
        assert!(matches!(g.admit(2, true, 1), Admission::Full(2)));
    }

    #[test]
    fn no_message_is_lost_or_duplicated_around_authentication() {
        for _ in 0..200 {
            let g = Arc::new(gate(false));
            let producers: Vec<_> = (0..4u32)
                .map(|p| {
                    let g = g.clone();
                    thread::spawn(move || {
                        (0..50u32)
                            .filter_map(|i| delivered(g.admit(p * 1000 + i, true, 1)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();

            let replayed = g.authenticate();

            let mut seen: Vec<u32> = replayed;
            for p in producers {
                seen.extend(p.join().unwrap());
            }
            // Anything admitted after the flag flipped was delivered directly; the rest was held.
            seen.extend(g.authenticate());

            let unique: HashSet<_> = seen.iter().copied().collect();
            assert_eq!(seen.len(), 200, "a message was lost or duplicated");
            assert_eq!(unique.len(), 200);
        }
    }
}
