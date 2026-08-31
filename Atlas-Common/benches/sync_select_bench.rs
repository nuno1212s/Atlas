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
use atlas_common::sync_select;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

/// Which backend `sync_select!` was compiled against in this build.
#[cfg(not(feature = "channel_sync_flume"))]
const ATLAS_BACKEND: &str = "crossbeam";
#[cfg(feature = "channel_sync_flume")]
const ATLAS_BACKEND: &str = "flume";

const CAPACITY: usize = 1024;

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

fn atlas_select_2(rx: &[ChannelSyncRx<u64>]) -> u64 {
    sync_select! {
        recv(rx[0]) -> m => m.unwrap(),
        recv(rx[1]) -> m => m.unwrap(),
    }
}

fn atlas_select_7(rx: &[ChannelSyncRx<u64>]) -> u64 {
    sync_select! {
        recv(rx[0]) -> m => m.unwrap(),
        recv(rx[1]) -> m => m.unwrap(),
        recv(rx[2]) -> m => m.unwrap(),
        recv(rx[3]) -> m => m.unwrap(),
        recv(rx[4]) -> m => m.unwrap(),
        recv(rx[5]) -> m => m.unwrap(),
        recv(rx[6]) -> m => m.unwrap(),
    }
}

/// Drains a backlog through a single `recv_exhaust` arm — the form 12 of the
/// tree's ~20 real arms use.
fn atlas_drain(rx: &ChannelSyncRx<u64>, sink: &mut u64) -> Result<(), RecvError> {
    sync_select! {
        recv_exhaust(*rx) -> v => {
            *sink = sink.wrapping_add(v);
            // Annotated because the desugaring applies `?` to this body; real
            // call sites pass a method call whose error type is already fixed.
            Ok::<(), RecvError>(())
        }
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

        let (atx, arx) = atlas_channels(arms);
        group.bench_with_input(
            BenchmarkId::new(format!("atlas_{ATLAS_BACKEND}"), arms),
            &arms,
            |b, &arms| {
                let mut i = 0usize;
                b.iter(|| {
                    atx[i % arms].send(1).unwrap();
                    i += 1;
                    black_box(if arms == 2 {
                        atlas_select_2(&arx)
                    } else {
                        atlas_select_7(&arx)
                    })
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
                        atlas_drain(&rx, &mut sink).unwrap();
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
    bench_send_only,
    bench_recv_exhaust
);
criterion_main!(benches);
