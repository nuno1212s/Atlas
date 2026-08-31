//! Behaviour of [`atlas_common::sync_select!`].
//!
//! These run under whichever sync channel backend is compiled in, so the same
//! expectations are checked for both crossbeam and flume.

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

    fn tick(&mut self) -> Result<(), RecvError> {
        atlas_common::sync_select! {
            recv_exhaust(self.a) -> v => self.on_a(v),
            recv_exhaust(self.b_rx()) -> v => self.on_b(v),
            recv_exhaust(self.pair.1) -> v => self.on_c(v),
            default(Duration::from_millis(50)) => Ok(()),
        }
    }

    /// Block bodies with no separating commas, as Atlas-Reconfiguration writes them.
    fn tick_blocks(&mut self) -> Result<(), RecvError> {
        atlas_common::sync_select! {
            recv(self.a) -> msg => {
                let v = msg?;
                self.on_a(v)
            }
            recv(self.b) -> msg => {
                let v = msg?;
                self.on_b(v)
            }
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
    let (ta, a) = new_bounded_sync(16, Some("a"));
    let (tb, b) = new_bounded_sync(16, Some("b"));
    let (tc, c) = new_bounded_sync(16, Some("c"));

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

#[test]
fn runs_only_the_ready_arm() {
    let (ta, _tb, _tc, mut h) = harness();
    ta.send(1).unwrap();

    h.tick().unwrap();

    assert_eq!(h.log, vec!["a1"]);
}

#[test]
fn recv_exhaust_drains_the_backlog() {
    let (ta, _tb, _tc, mut h) = harness();
    ta.send(1).unwrap();
    ta.send(2).unwrap();
    ta.send(3).unwrap();

    h.tick().unwrap();

    // One select, but every queued message is consumed.
    assert_eq!(h.log, vec!["a1", "a2", "a3"]);
}

#[test]
fn each_arm_is_reachable_across_repeated_calls() {
    // Guards against a slot keeping a stale value and winning every time.
    let (ta, tb, tc, mut h) = harness();

    for (send, expected) in [(0, "a9"), (1, "b9"), (2, "c9"), (1, "b9"), (0, "a9")] {
        h.log.clear();
        match send {
            0 => ta.send(9).unwrap(),
            1 => tb.send(9).unwrap(),
            _ => tc.send(9).unwrap(),
        }

        h.tick().unwrap();
        assert_eq!(h.log, vec![expected], "arm {send} should have won");
    }
}

#[test]
fn default_arm_runs_when_nothing_is_ready() {
    let (_ta, _tb, _tc, mut h) = harness();

    h.tick().unwrap();

    assert!(h.log.is_empty());
}

#[test]
fn block_bodies_without_commas_parse() {
    let (ta, _tb, _tc, mut h) = harness();
    ta.send(42).unwrap();

    h.tick_blocks().unwrap();

    assert_eq!(h.log, vec!["a42"]);
}

#[test]
fn binding_carries_the_channel_name_on_disconnect() {
    // The backend's own error type never reaches here, and the name that
    // `new_bounded_sync` was given is already attached.
    let (tx, rx) = new_bounded_sync::<u32>(4, Some("named-channel"));
    drop(tx);

    let err = atlas_common::sync_select! {
        recv(rx) -> msg => msg,
    }
    .expect_err("a dropped sender must surface as a disconnect");

    let RecvError::ChannelDc { channel } = err;
    assert_eq!(channel.as_deref(), Some("named-channel"));
}

#[test]
fn accepts_a_reference_receiver() {
    let (tx, rx) = new_bounded_sync::<u32>(4, Some("r"));
    tx.send(5).unwrap();
    let by_ref: &ChannelSyncRx<u32> = &rx;

    let got = atlas_common::sync_select! {
        recv(by_ref) -> msg => msg,
    };

    assert_eq!(got.unwrap(), 5);
}
