//! Cost of `atlas_common::sync_select!` relative to the backends it wraps.
//!
//! Three things are compared:
//!
//! 1. `raw_crossbeam` — `crossbeam_channel::select!` used directly.
//! 2. `atlas_<backend>` — our `sync_select!`, over whichever backend this build
//!    selected.
//! 3. `raw_flume` — `flume::Selector` used directly.
//!
//! `sync_select!` resolves to exactly one backend per build, so a single binary
//! can only measure one of `atlas_crossbeam` / `atlas_flume`. Run it twice:
//!
//! ```text
//! cargo bench --bench sync_select_bench
//! cargo bench --bench sync_select_bench --features channel_sync_flume
//! ```
//!
//! The two raw variants are measured in both runs and should land on the same
//! numbers each time — they are the calibration that makes the two runs
//! comparable.
//!
//! # Reading the numbers
//!
//! Each timed iteration is *one send plus one select-and-consume*, keeping
//! exactly one message in flight so that exactly one arm is ready — the busy-loop
//! case the replica actually hits. The send is included because it cannot be
//! removed without per-iteration timer calls that would cost as much as the
//! operation being measured; the `send_only` group measures it in isolation so it
//! can be subtracted.
//!
//! For the headline question — what our mapping costs — compare `raw_crossbeam`
//! against `atlas_crossbeam` directly. Those two push messages through the *same*
//! channel implementation, so the send cancels out and the difference is the
//! wrapper alone.

use atlas_common::channel::RecvError;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx, new_bounded_sync};
use atlas_common::{sync_drain, sync_select};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;

/// Which backend `sync_select!` was compiled against in this build.
#[cfg(not(feature = "channel_sync_flume"))]
const ATLAS_BACKEND: &str = "crossbeam";
#[cfg(feature = "channel_sync_flume")]
const ATLAS_BACKEND: &str = "flume";

const CAPACITY: usize = 1024;

/// The `default(..)` the replica's main loop selects with. It never fires in
/// these benches — a message is always waiting — so what it measures is what the
/// timeout *form* costs on the busy path.
const TIMEOUT: Duration = Duration::from_millis(1);

// Arm counts mirror the real call sites: the narrow workers select over 2, the
// replica's main loop over 7.
// ---------------------------------------------------------------- raw crossbeam

fn crossbeam_channels(
    n: usize,
) -> (
    Vec<crossbeam_channel::Sender<u64>>,
    Vec<crossbeam_channel::Receiver<u64>>,
) {
    (0..n).map(|_| crossbeam_channel::bounded(CAPACITY)).unzip()
}

fn crossbeam_select_2(rx: &[crossbeam_channel::Receiver<u64>]) -> u64 {
    crossbeam_channel::select! {
        recv(&rx[0]) -> m => m.unwrap(),
        recv(&rx[1]) -> m => m.unwrap(),
    }
}

fn crossbeam_select_7(rx: &[crossbeam_channel::Receiver<u64>]) -> u64 {
    crossbeam_channel::select! {
        recv(&rx[0]) -> m => m.unwrap(),
        recv(&rx[1]) -> m => m.unwrap(),
        recv(&rx[2]) -> m => m.unwrap(),
        recv(&rx[3]) -> m => m.unwrap(),
        recv(&rx[4]) -> m => m.unwrap(),
        recv(&rx[5]) -> m => m.unwrap(),
        recv(&rx[6]) -> m => m.unwrap(),
    }
}

/// The same index-based `Select` builder our macro drives, hand-written without
/// the per-arm `Option` slots. Sits between `raw_crossbeam` (the `select!` macro,
/// which knows its arity at compile time and uses a stack array) and
/// `atlas_crossbeam`, so the two gaps attribute the cost: macro-vs-builder on one
/// side, our wrapper on the other.
fn crossbeam_builder(rx: &[crossbeam_channel::Receiver<u64>]) -> u64 {
    let mut sel = crossbeam_channel::Select::new();

    for r in rx {
        sel.recv(r);
    }

    let op = sel.select();
    let idx = op.index();

    op.recv(&rx[idx]).unwrap()
}

fn crossbeam_select_timeout_2(rx: &[crossbeam_channel::Receiver<u64>]) -> u64 {
    crossbeam_channel::select! {
        recv(&rx[0]) -> m => m.unwrap(),
        recv(&rx[1]) -> m => m.unwrap(),
        default(TIMEOUT) => 0,
    }
}

fn crossbeam_select_timeout_7(rx: &[crossbeam_channel::Receiver<u64>]) -> u64 {
    crossbeam_channel::select! {
        recv(&rx[0]) -> m => m.unwrap(),
        recv(&rx[1]) -> m => m.unwrap(),
        recv(&rx[2]) -> m => m.unwrap(),
        recv(&rx[3]) -> m => m.unwrap(),
        recv(&rx[4]) -> m => m.unwrap(),
        recv(&rx[5]) -> m => m.unwrap(),
        recv(&rx[6]) -> m => m.unwrap(),
        default(TIMEOUT) => 0,
    }
}

fn crossbeam_builder_timeout(rx: &[crossbeam_channel::Receiver<u64>]) -> u64 {
    let mut sel = crossbeam_channel::Select::new();

    for r in rx {
        sel.recv(r);
    }

    match sel.select_timeout(TIMEOUT) {
        Ok(op) => {
            let idx = op.index();

            op.recv(&rx[idx]).unwrap()
        }
        Err(_) => 0,
    }
}

/// `crossbeam_builder` with the per-call fairness shuffle turned off.
///
/// `run_select` shuffles its handle list on every call unless the selector is
/// biased, and `select!` is always unbiased — so the gap between this and
/// `crossbeam_builder` is a cost the macro cannot avoid and we can. The bench
/// keeps exactly one message in flight, so no arm is ever contended and bias
/// cannot change *which* arm wins: the difference is the shuffle alone.
fn crossbeam_builder_biased(rx: &[crossbeam_channel::Receiver<u64>]) -> u64 {
    let mut sel = crossbeam_channel::Select::new_biased();

    for r in rx {
        sel.recv(r);
    }

    let op = sel.select();
    let idx = op.index();

    op.recv(&rx[idx]).unwrap()
}

// --------------------------------------------------------------------- raw flume

fn flume_channels(n: usize) -> (Vec<flume::Sender<u64>>, Vec<flume::Receiver<u64>>) {
    (0..n).map(|_| flume::bounded(CAPACITY)).unzip()
}

fn flume_select_2(rx: &[flume::Receiver<u64>]) -> u64 {
    flume::Selector::new()
        .recv(&rx[0], |m| m.unwrap())
        .recv(&rx[1], |m| m.unwrap())
        .wait()
}

fn flume_select_7(rx: &[flume::Receiver<u64>]) -> u64 {
    flume::Selector::new()
        .recv(&rx[0], |m| m.unwrap())
        .recv(&rx[1], |m| m.unwrap())
        .recv(&rx[2], |m| m.unwrap())
        .recv(&rx[3], |m| m.unwrap())
        .recv(&rx[4], |m| m.unwrap())
        .recv(&rx[5], |m| m.unwrap())
        .recv(&rx[6], |m| m.unwrap())
        .wait()
}

// ------------------------------------------------------------ atlas sync_select!

fn atlas_channels(n: usize) -> (Vec<ChannelSyncTx<u64>>, Vec<ChannelSyncRx<u64>>) {
    (0..n)
        .map(|i| new_bounded_sync(CAPACITY, Some(format!("bench-{i}"))))
        .unzip()
}

// `sync_select!` is `sync_drain!` (one bounded round over every arm, running
// bodies as messages are taken) followed by a parker. Each half is measured on
// its own below, plus the composition, so a movement can be attributed to one of
// them rather than to "the macro".

fn atlas_drain_2(rx: &[ChannelSyncRx<u64>], sink: &mut u64) -> Result<(), RecvError> {
    sync_drain! {
        recv(rx[0]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[1]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
    }
}

fn atlas_drain_7(rx: &[ChannelSyncRx<u64>], sink: &mut u64) -> Result<(), RecvError> {
    sync_drain! {
        recv(rx[0]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[1]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[2]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[3]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[4]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[5]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[6]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
    }
}

fn atlas_select_2(rx: &[ChannelSyncRx<u64>], sink: &mut u64) -> Result<(), RecvError> {
    sync_select! {
        recv(rx[0]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[1]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        default(TIMEOUT) => Ok(())
    }
}

fn atlas_select_7(rx: &[ChannelSyncRx<u64>], sink: &mut u64) -> Result<(), RecvError> {
    sync_select! {
        recv(rx[0]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[1]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[2]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[3]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[4]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[5]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        recv(rx[6]) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
        default(TIMEOUT) => Ok(())
    }
}

/// Drains a backlog through a single arm — the amortization `recv_exhaust` was
/// built for, now the default behaviour of every arm.
fn atlas_drain_one(rx: &ChannelSyncRx<u64>, sink: &mut u64) -> Result<(), RecvError> {
    sync_drain! {
        recv(*rx) -> v => { *sink = sink.wrapping_add(v); Ok::<(), RecvError>(()) }
    }
}

// ------------------------------------------------------------------------ benches

fn bench_select_one_ready(c: &mut Criterion) {
    let mut group = c.benchmark_group("select_one_ready");
    group.throughput(Throughput::Elements(1));

    for &arms in &[2usize, 7] {
        // Rotating the target channel keeps every arm exercised rather than
        // letting one hot arm dominate the branch predictor.
        let (ctx, crx) = crossbeam_channels(arms);
        group.bench_with_input(
            BenchmarkId::new("raw_crossbeam", arms),
            &arms,
            |b, &arms| {
                let mut i = 0usize;
                b.iter(|| {
                    ctx[i % arms].send(1).unwrap();
                    i += 1;
                    black_box(if arms == 2 {
                        crossbeam_select_2(&crx)
                    } else {
                        crossbeam_select_7(&crx)
                    })
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("raw_crossbeam_builder", arms),
            &arms,
            |b, &arms| {
                let mut i = 0usize;
                b.iter(|| {
                    ctx[i % arms].send(1).unwrap();
                    i += 1;
                    black_box(crossbeam_builder(&crx))
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("raw_crossbeam_builder_biased", arms),
            &arms,
            |b, &arms| {
                let mut i = 0usize;
                b.iter(|| {
                    ctx[i % arms].send(1).unwrap();
                    i += 1;
                    black_box(crossbeam_builder_biased(&crx))
                })
            },
        );

        let (ftx, frx) = flume_channels(arms);
        group.bench_with_input(BenchmarkId::new("raw_flume", arms), &arms, |b, &arms| {
            let mut i = 0usize;
            b.iter(|| {
                ftx[i % arms].send(1).unwrap();
                i += 1;
                black_box(if arms == 2 {
                    flume_select_2(&frx)
                } else {
                    flume_select_7(&frx)
                })
            })
        });
    }

    group.finish();
}

/// `select_one_ready` in the `default(..)` form, which is what the replica's main
/// loop actually selects with.
///
/// A message is always waiting, so the timeout never fires and every iteration
/// takes the same path through the channels as `select_one_ready` does. Whatever
/// separates the two groups is what asking for a timeout costs on the busy path —
/// `select_timeout` reads the clock before it polls anything.
fn bench_select_timeout(c: &mut Criterion) {
    let mut group = c.benchmark_group("select_timeout_one_ready");
    group.throughput(Throughput::Elements(1));

    for &arms in &[2usize, 7] {
        let (ctx, crx) = crossbeam_channels(arms);
        group.bench_with_input(
            BenchmarkId::new("raw_crossbeam", arms),
            &arms,
            |b, &arms| {
                let mut i = 0usize;
                b.iter(|| {
                    ctx[i % arms].send(1).unwrap();
                    i += 1;
                    black_box(if arms == 2 {
                        crossbeam_select_timeout_2(&crx)
                    } else {
                        crossbeam_select_timeout_7(&crx)
                    })
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("raw_crossbeam_builder", arms),
            &arms,
            |b, &arms| {
                let mut i = 0usize;
                b.iter(|| {
                    ctx[i % arms].send(1).unwrap();
                    i += 1;
                    black_box(crossbeam_builder_timeout(&crx))
                })
            },
        );
    }

    group.finish();
}

/// Component 1, one arm ready: the round finds a message in one arm and scans
/// the rest.
///
/// This is what a loop keeping up with its inbox does on nearly every iteration,
/// and the reason `sync_select!` runs a round before it parks on anything.
fn bench_round_one_ready(c: &mut Criterion) {
    let mut group = c.benchmark_group("round_one_ready");
    group.throughput(Throughput::Elements(1));

    for &arms in &[2usize, 7] {
        let (atx, arx) = atlas_channels(arms);
        group.bench_with_input(
            BenchmarkId::new(format!("atlas_{ATLAS_BACKEND}"), arms),
            &arms,
            |b, &arms| {
                let mut sink = 0u64;
                let mut i = 0usize;
                b.iter(|| {
                    atx[i % arms].send(1).unwrap();
                    i += 1;
                    if arms == 2 {
                        atlas_drain_2(&arx, &mut sink).unwrap()
                    } else {
                        atlas_drain_7(&arx, &mut sink).unwrap()
                    }
                    black_box(sink)
                })
            },
        );
    }

    group.finish();
}

/// Component 1, every arm ready: the shape the round exists for.
///
/// One round services all of them; the early-exit design it replaced would have
/// needed one round-trip per arm. Throughput counts every message, so this is
/// directly comparable per-message with `round_one_ready`.
fn bench_round_all_ready(c: &mut Criterion) {
    let mut group = c.benchmark_group("round_all_ready");

    for &arms in &[2usize, 7] {
        group.throughput(Throughput::Elements(arms as u64));

        let (atx, arx) = atlas_channels(arms);
        group.bench_with_input(
            BenchmarkId::new(format!("atlas_{ATLAS_BACKEND}"), arms),
            &arms,
            |b, &arms| {
                let mut sink = 0u64;
                b.iter(|| {
                    for tx in atx.iter().take(arms) {
                        tx.send(1).unwrap();
                    }
                    if arms == 2 {
                        atlas_drain_2(&arx, &mut sink).unwrap()
                    } else {
                        atlas_drain_7(&arx, &mut sink).unwrap()
                    }
                    black_box(sink)
                })
            },
        );
    }

    group.finish();
}

/// Component 1, nothing ready: a full scan that finds nothing and returns.
///
/// No send, so what is timed is only the scan. This is the price `sync_select!`
/// pays before it parks — the one case where running a round first is pure loss.
fn bench_round_none_ready(c: &mut Criterion) {
    let mut group = c.benchmark_group("round_none_ready");
    group.throughput(Throughput::Elements(1));

    for &arms in &[2usize, 7] {
        let (_atx, arx) = atlas_channels(arms);
        group.bench_with_input(
            BenchmarkId::new(format!("atlas_{ATLAS_BACKEND}"), arms),
            &arms,
            |b, &arms| {
                let mut sink = 0u64;
                b.iter(|| {
                    if arms == 2 {
                        atlas_drain_2(&arx, &mut sink).unwrap()
                    } else {
                        atlas_drain_7(&arx, &mut sink).unwrap()
                    }
                    black_box(sink)
                })
            },
        );
    }

    group.finish();
}

/// Both halves composed — `sync_select!` as call sites write it, in the
/// `default(..)` form the replica's main loop uses.
///
/// A message is always waiting, so the round finds it and the parker never runs.
/// Against `round_one_ready` this is what composing the two costs.
fn bench_select(c: &mut Criterion) {
    let mut group = c.benchmark_group("select_one_ready_adaptive");
    group.throughput(Throughput::Elements(1));

    for &arms in &[2usize, 7] {
        let (atx, arx) = atlas_channels(arms);
        group.bench_with_input(
            BenchmarkId::new(format!("atlas_{ATLAS_BACKEND}"), arms),
            &arms,
            |b, &arms| {
                let mut sink = 0u64;
                let mut i = 0usize;
                b.iter(|| {
                    atx[i % arms].send(1).unwrap();
                    i += 1;
                    if arms == 2 {
                        atlas_select_2(&arx, &mut sink).unwrap()
                    } else {
                        atlas_select_7(&arx, &mut sink).unwrap()
                    }
                    black_box(sink)
                })
            },
        );
    }

    group.finish();
}

/// The send that `select_one_ready` includes, measured on its own so it can be
/// subtracted when comparing across backends.
fn bench_send_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("send_only");
    group.throughput(Throughput::Elements(1));

    let (ctx, crx) = crossbeam_channels(1);
    group.bench_function("crossbeam", |b| {
        b.iter(|| {
            ctx[0].send(1).unwrap();
            black_box(crx[0].recv().unwrap())
        })
    });

    let (ftx, frx) = flume_channels(1);
    group.bench_function("flume", |b| {
        b.iter(|| {
            ftx[0].send(1).unwrap();
            black_box(frx[0].recv().unwrap())
        })
    });

    let (atx, arx) = atlas_channels(1);
    group.bench_function(format!("atlas_{ATLAS_BACKEND}"), |b| {
        b.iter(|| {
            atx[0].send(1).unwrap();
            black_box(arx[0].recv().unwrap())
        })
    });

    group.finish();
}

/// `recv_exhaust` amortized over a backlog: one select, then a `try_recv` drain.
fn bench_recv_exhaust(c: &mut Criterion) {
    let mut group = c.benchmark_group("recv_exhaust_drain");

    for &backlog in &[1usize, 16, 64] {
        group.throughput(Throughput::Elements(backlog as u64));

        let (tx, rx) = new_bounded_sync::<u64>(CAPACITY, Some("drain"));
        group.bench_with_input(
            BenchmarkId::new(format!("atlas_{ATLAS_BACKEND}"), backlog),
            &backlog,
            |b, &backlog| {
                let mut sink = 0u64;
                b.iter_batched(
                    // PerIteration keeps setup and routine strictly alternating,
                    // so the drain sees exactly `backlog` messages each time.
                    || {
                        for _ in 0..backlog {
                            tx.send(1).unwrap();
                        }
                    },
                    |()| {
                        atlas_drain_one(&rx, &mut sink).unwrap();
                        black_box(sink)
                    },
                    criterion::BatchSize::PerIteration,
                )
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_select_one_ready,
    bench_select_timeout,
    bench_round_one_ready,
    bench_round_all_ready,
    bench_round_none_ready,
    bench_select,
    bench_send_only,
    bench_recv_exhaust
);
criterion_main!(benches);
