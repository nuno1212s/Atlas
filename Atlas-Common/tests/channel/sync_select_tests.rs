//! Behaviour of [`atlas_common::sync_drain!`] and [`atlas_common::sync_select!`].
//!
//! These run under whichever sync channel backend is compiled in, so the same
//! expectations are checked for both crossbeam and flume.
//!
//! `sync_select!` is `sync_drain!` (one bounded round over every arm) followed by
//! a parker, so each half is exercised on its own here — a failure then says
//! which one broke.

use atlas_common::channel::RecvError;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx, new_bounded_sync};
use std::time::Duration;

/// Stands in for the real call sites: channels reached both as fields and
/// through getters, with every arm body taking `&mut self`.
struct Harness {
    a: ChannelSyncRx<u32>,
    b: ChannelSyncRx<u32>,
    pair: (u8, ChannelSyncRx<u32>),
    log: Vec<String>,
}

impl Harness {
    fn b_rx(&self) -> &ChannelSyncRx<u32> {
        &self.b
    }

    fn on_a(&mut self, v: u32) -> Result<(), RecvError> {
        self.log.push(format!("a{v}"));
        Ok(())
    }

    fn on_b(&mut self, v: u32) -> Result<(), RecvError> {
        self.log.push(format!("b{v}"));
        Ok(())
    }

    fn on_c(&mut self, v: u32) -> Result<(), RecvError> {
        self.log.push(format!("c{v}"));
        Ok(())
    }

    /// Component 1 alone: one bounded round, no parking.
    fn drain_round(&mut self) -> Result<(), RecvError> {
        atlas_common::sync_drain! {
            recv(self.a) -> v => self.on_a(v),
            recv(self.b_rx()) -> v => self.on_b(v),
            recv(self.pair.1) -> v => self.on_c(v),
        }
    }

    /// Component 1 with the per-call cap lowered.
    fn drain_round_capped(&mut self) -> Result<(), RecvError> {
        atlas_common::sync_drain! {
            limit(2);
            recv(self.a) -> v => self.on_a(v),
            recv(self.b_rx()) -> v => self.on_b(v),
            recv(self.pair.1) -> v => self.on_c(v),
        }
    }

    /// Both halves, as call sites write it. Block bodies with no separating
    /// commas, as Atlas-Reconfiguration writes them.
    fn tick(&mut self) -> Result<(), RecvError> {
        atlas_common::sync_select! {
            recv(self.a) -> v => { self.on_a(v) }
            recv_exhaust(self.b_rx()) -> v => { self.on_b(v) }
            recv(self.pair.1) -> v => { self.on_c(v) }
            default(Duration::from_millis(50)) => Ok(())
        }
    }
}

fn harness() -> (
    ChannelSyncTx<u32>,
    ChannelSyncTx<u32>,
    ChannelSyncTx<u32>,
    Harness,
) {
    let (ta, a) = new_bounded_sync(512, Some("a"));
    let (tb, b) = new_bounded_sync(512, Some("b"));
    let (tc, c) = new_bounded_sync(512, Some("c"));

    (
        ta,
        tb,
        tc,
        Harness {
            a,
            b,
            pair: (0, c),
            log: vec![],
        },
    )
}

// ---------------------------------------------------------------- the round

#[test]
fn a_round_drains_the_arm_it_visits() {
    let (ta, _tb, _tc, mut h) = harness();
    ta.send(1).unwrap();
    ta.send(2).unwrap();
    ta.send(3).unwrap();

    h.drain_round().unwrap();

    assert_eq!(h.log, vec!["a1", "a2", "a3"]);
}

#[test]
fn a_round_visits_every_arm() {
    // The property arm ordering used to be responsible for: a busy arm cannot
    // keep the arms after it from being served, because the round reaches them
    // in the same pass.
    let (ta, tb, tc, mut h) = harness();
    for _ in 0..4 {
        ta.send(1).unwrap();
    }
    tb.send(2).unwrap();
    tc.send(3).unwrap();

    h.drain_round().unwrap();

    assert_eq!(h.log, vec!["a1", "a1", "a1", "a1", "b2", "c3"]);
}

#[test]
fn a_round_over_empty_channels_does_nothing_and_returns() {
    let (_ta, _tb, _tc, mut h) = harness();

    // Hanging here is the failure this guards against: a round never blocks.
    h.drain_round().unwrap();

    assert!(h.log.is_empty());
}

#[test]
fn the_limit_caps_what_one_arm_takes_per_round() {
    let (ta, _tb, _tc, mut h) = harness();
    for i in 1..=5 {
        ta.send(i).unwrap();
    }

    h.drain_round_capped().unwrap();
    assert_eq!(h.log, vec!["a1", "a2"], "one round should take at most 2");

    // The rest is still queued, and comes out over the following rounds.
    h.log.clear();
    h.drain_round_capped().unwrap();
    assert_eq!(h.log, vec!["a3", "a4"]);

    h.log.clear();
    h.drain_round_capped().unwrap();
    assert_eq!(h.log, vec!["a5"]);
}

#[test]
fn an_erroring_body_ends_the_round_without_taking_from_later_arms() {
    // Nothing is dequeued until its body is about to run, so the messages the
    // round never reached must still be in their channels afterwards.
    let (ta, tb, _tc, mut h) = harness();
    ta.send(1).unwrap();
    tb.send(2).unwrap();

    let failed = atlas_common::sync_drain! {
        recv(h.a) -> _v => Err::<(), _>(RecvError::ChannelDc { channel: None }),
        recv(h.b) -> v => h.on_b(v),
    };

    assert!(failed.is_err());
    assert!(h.log.is_empty(), "the second arm's body must not have run");
    assert_eq!(
        h.b.try_recv().unwrap(),
        2,
        "the second arm's message must still be queued"
    );
}

#[test]
fn a_disconnect_surfaces_as_an_error_carrying_the_channel_name() {
    let (tx, rx) = new_bounded_sync::<u32>(4, Some("hung-up"));
    drop(tx);

    let err = atlas_common::sync_drain! {
        recv(rx) -> _v => Ok::<(), RecvError>(()),
    }
    .expect_err("a dropped sender must surface as a disconnect");

    let RecvError::ChannelDc { channel } = err;
    assert_eq!(channel.as_deref(), Some("hung-up"));
}

#[test]
fn a_round_accepts_a_reference_receiver() {
    let (tx, rx) = new_bounded_sync::<u32>(4, Some("r"));
    tx.send(5).unwrap();
    let by_ref: &ChannelSyncRx<u32> = &rx;
    let mut seen = vec![];

    atlas_common::sync_drain! {
        recv(by_ref) -> v => { seen.push(v); Ok::<(), RecvError>(()) }
    }
    .unwrap();

    assert_eq!(seen, vec![5]);
}

// ------------------------------------------------------- round plus parking

#[test]
fn the_default_arm_runs_when_the_deadline_passes() {
    let (_ta, _tb, _tc, mut h) = harness();

    let started = std::time::Instant::now();
    h.tick().unwrap();

    assert!(h.log.is_empty());
    assert!(
        started.elapsed() >= Duration::from_millis(40),
        "expected to park until the deadline, took {:?}",
        started.elapsed()
    );
}

#[test]
fn parking_wakes_for_a_message_that_arrives_late() {
    // Every channel is empty when the first round runs, so a message sent only
    // afterwards can only be seen once the parker has woken the loop.
    let (ta, _tb, _tc, mut h) = harness();

    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        ta.send(11).unwrap();
    });

    h.tick().unwrap();

    sender.join().unwrap();
    assert_eq!(h.log, vec!["a11"]);
}

#[test]
fn a_ready_arm_returns_without_parking() {
    let (_ta, _tb, tc, mut h) = harness();
    tc.send(7).unwrap();

    let started = std::time::Instant::now();
    h.tick().unwrap();

    assert_eq!(h.log, vec!["c7"]);
    assert!(
        started.elapsed() < Duration::from_millis(20),
        "the round found a message, so nothing should have parked"
    );
}

#[test]
fn each_arm_is_reachable_across_repeated_calls() {
    // Guards against an arm being skipped once it has been served.
    let (ta, tb, tc, mut h) = harness();

    for (send, expected) in [(0, "a9"), (1, "b9"), (2, "c9"), (1, "b9"), (0, "a9")] {
        h.log.clear();
        match send {
            0 => ta.send(9).unwrap(),
            1 => tb.send(9).unwrap(),
            _ => tc.send(9).unwrap(),
        }

        h.tick().unwrap();
        assert_eq!(h.log, vec![expected], "arm {send} should have been served");
    }
}
