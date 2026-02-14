use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use atlas_common::ordering::{SeqNo};
use atlas_common::ordering::tbo_queue::TTboQueue;
use atlas_common::ordering::Orderable;
use std::sync::Arc;
use atlas_common::ordering::tbo_queue::tbo_queue::TboQueue;
use atlas_common::ordering::tbo_queue::vec_tbo_queue::VTboQueue;

// A tiny message type implementing Orderable to use in benchmarks and as an example.
#[derive(Clone)]
struct BenchMsg {
    seq: SeqNo,
    payload: Arc<Vec<u8>>,
}

impl Orderable for BenchMsg {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl BenchMsg {
    fn new(seq: SeqNo, size: usize) -> Self {
        BenchMsg { seq, payload: Arc::new(vec![0u8; size]) }
    }
}

// Contract for the generic bench helpers:
// - F: factory that returns a fresh queue instance (T)
// - G: message generator: Fn(usize) -> M
// - T: queue type implementing TTboQueue<M>
// - M: message type implementing Orderable + Clone

fn bench_push<F, G, T, M>(c: &mut Criterion, id: &str, factory: F, gen: G, sizes: &[usize])
where
    F: Fn() -> T + Send + Sync + 'static,
    G: Fn(usize) -> M + Send + Sync + 'static,
    T: TTboQueue<M> + Send + 'static,
    M: Orderable + Clone + Send + 'static,
{
    let mut group = c.benchmark_group(format!("tbo_push_{}", id));

    for &size in sizes {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            b.iter_batched(
                || factory(),
                |mut q| {
                    for i in 0..n {
                        // create an in-order message by default
                        let m = gen(i);
                        let _ = q.push(m);
                    }
                },
                criterion::BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

fn bench_pop<F, G, T, M>(c: &mut Criterion, id: &str, factory: F, gen: G, sizes: &[usize])
where
    F: Fn() -> T + Send + Sync + 'static,
    G: Fn(usize) -> M + Send + Sync + 'static,
    T: TTboQueue<M> + Send + 'static,
    M: Orderable + Clone + Send + 'static,
{
    let mut group = c.benchmark_group(format!("tbo_pop_{}", id));

    for &size in sizes {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            b.iter_batched(
                || {
                    // prepare a queue with n in-order messages
                    let mut q = factory();
                    for i in 0..n {
                        let _ = q.push(gen(i));
                    }
                    q
                },
                |mut q| {
                    let mut cnt = 0usize;
                    while let Some(_m) = q.pop() {
                        cnt += 1;
                        q.advance_seq();
                    }
                    debug_assert_eq!(cnt, n);
                },
                criterion::BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

fn bench_peek<F, G, T, M>(c: &mut Criterion, id: &str, factory: F, gen: G, sizes: &[usize])
where
    F: Fn() -> T + Send + Sync + 'static,
    G: Fn(usize) -> M + Send + Sync + 'static,
    T: TTboQueue<M> + Send + 'static,
    M: Orderable + Clone + Send + 'static,
{
    let mut group = c.benchmark_group(format!("tbo_peek_{}", id));

    for &size in sizes {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            b.iter_batched(
                || {
                    let mut q = factory();
                    for i in 0..n {
                        let _ = q.push(gen(i));
                    }
                    q
                },
                |mut q| {
                    // repeatedly peek and advance the sequence so peek returns None eventually
                    while let Some(_m) = q.peek() {
                        let _ = q.peek();
                        q.pop();
                        q.advance_seq();
                    }
                },
                criterion::BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

fn bench_advance_install_clear<F, G, T, M>(c: &mut Criterion, id: &str, factory: F, gen: G)
where
    F: Fn() -> T + Send + Sync + 'static,
    G: Fn(usize) -> M + Send + Sync + 'static,
    T: TTboQueue<M> + Send + 'static,
    M: Orderable + Clone + Send + 'static,
{
    let mut group = c.benchmark_group(format!("tbo_misc_{}", id));

    group.bench_function("advance_seq_many", |b| {
        b.iter_batched(
            || {
                let mut q = factory();
                for i in 0..1000usize {
                    let _ = q.push(gen(i));
                }
                q
            },
            |mut q| {
                for _ in 0..1000usize {
                    q.advance_seq();
                }
            },
            criterion::BatchSize::LargeInput,
        )
    });

    group.bench_function("install_seq_skip", |b| {
        b.iter_batched(
            || {
                let mut q = factory();
                for i in 0..1000usize {
                    let _ = q.push(gen(i));
                }
                q
            },
            |mut q| {
                q.install_seq(SeqNo::from(500u32));
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.bench_function("clear_queue", |b| {
        b.iter_batched(
            || {
                let mut q = factory();
                for i in 0..10000usize {
                    let _ = q.push(gen(i));
                }
                q
            },
            |mut q| {
                q.clear();
            },
            criterion::BatchSize::LargeInput,
        )
    });

    group.finish();
}

// Example: bench the library's TboQueue implementation with varying payload sizes
fn tbo_queue_bench(c: &mut Criterion) {
    let sizes = [1usize, 10usize, 100usize, 1000usize];

    // factory produces a fresh TboQueue<BenchMsg>
    let factory = || TboQueue::<BenchMsg>::new();
    let gen = |i: usize| BenchMsg::new(SeqNo::from(i as u32), 128);

    bench_push(c, "tbo_btree", factory, gen, &sizes);
    bench_pop(c, "tbo_btree", factory, gen, &sizes);
    bench_peek(c, "tbo_btree", factory, gen, &sizes);
    bench_advance_install_clear(c, "tbo_btree", factory, gen);
}

fn tbo_queue_vec_bench(c: &mut Criterion) {
    let sizes = [1usize, 10usize, 100usize, 1000usize];

    // factory produces a fresh VTboQueue<BenchMsg>
    let factory = || VTboQueue::<BenchMsg>::new();
    let gen = |i: usize| BenchMsg::new(SeqNo::from(i as u32), 128);

    bench_push(c, "tbo_vec", factory, gen, &sizes);
    bench_pop(c, "tbo_vec", factory, gen, &sizes);
    bench_peek(c, "tbo_vec", factory, gen, &sizes);
    bench_advance_install_clear(c, "tbo_vec", factory, gen);
}

criterion_group!(benches, tbo_queue_bench, tbo_queue_vec_bench);
criterion_main!(benches);