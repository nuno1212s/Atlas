use atlas_common::ordering::Orderable;
use atlas_common::ordering::SeqNo;
use atlas_common::ordering::tbo_queue::TTboQueue;
use atlas_common::ordering::tbo_queue::btree_tbo_queue::TboQueue;
use atlas_common::ordering::tbo_queue::vec_tbo_queue::VTboQueue;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::fmt::Display;

#[derive(Clone, Debug, Copy)]
struct BenchSeq(usize, usize);

/// 128 byte message payload to make the benchmarks more realistic.
const MSG_SIZE: usize = 128;

const MSG_CONTAINER: [u8; MSG_SIZE] = [1u8; MSG_SIZE];

impl Display for BenchSeq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "seq{}_msg_per_seq{}", self.0, self.1)
    }
}

// A tiny message type implementing Orderable to use in benchmarks and as an example.
#[derive(Clone)]
struct BenchMsg {
    seq: SeqNo,
    _p: [u8; MSG_SIZE],
}

impl Orderable for BenchMsg {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

impl BenchMsg {
    fn new(seq: SeqNo) -> Self {
        BenchMsg {
            seq,
            _p: MSG_CONTAINER,
        }
    }
}

// Contract for the generic bench helpers:
// - F: factory that returns a fresh queue instance (T)
// - G: message generator: Fn(usize, usize) -> M (takes message_index and msg_per_seq)
// - T: queue type implementing TTboQueue<M>
// - M: message type implementing Orderable + Clone
// - msg_per_seq: number of messages per sequence number (dynamic parameter)

fn bench_push<F, G, T, M>(
    c: &mut Criterion,
    id: &str,
    factory: F,
    generator: G,
    sizes: &[usize],
    msg_per_seqs: &[usize],
) where
    F: Fn() -> T + Send + Sync + 'static,
    G: Fn(usize, usize) -> M + Send + Sync + 'static,
    T: TTboQueue<M> + Send + 'static,
    M: Orderable + Clone + Send + 'static,
{
    let mut group = c.benchmark_group(format!("tbo_push_{id}"));

    for &size in sizes {
        for &msg_per_seq in msg_per_seqs {
            let total_messages = size * msg_per_seq;
            group.throughput(Throughput::Elements(total_messages as u64));
            let seq = BenchSeq(size, msg_per_seq);
            group.bench_with_input(BenchmarkId::from_parameter(seq), &seq, |b, &num_seqs| {
                b.iter_batched(
                    &factory,
                    |mut q| {
                        for i in 0..total_messages {
                            let m = generator(i, num_seqs.1);
                            let _ = q.push(m);
                        }
                    },
                    criterion::BatchSize::LargeInput,
                )
            });
        }
    }

    group.finish();
}

fn bench_pop<F, G, T, M>(
    c: &mut Criterion,
    id: &str,
    factory: F,
    generator: G,
    sizes: &[usize],
    msg_per_seqs: &[usize],
) where
    F: Fn() -> T + Send + Sync + 'static,
    G: Fn(usize, usize) -> M + Send + Sync + 'static,
    T: TTboQueue<M> + Send + 'static,
    M: Orderable + Clone + Send + 'static,
{
    let mut group = c.benchmark_group(format!("tbo_pop_{id}"));

    for &size in sizes {
        for &msg_per_seq in msg_per_seqs {
            let total_messages = size * msg_per_seq;
            group.throughput(Throughput::Elements(total_messages as u64));
            let seq = BenchSeq(size, msg_per_seq);
            group.bench_with_input(BenchmarkId::from_parameter(seq), &seq, |b, &num_seqs| {
                b.iter_batched(
                    || {
                        // prepare a queue with total_messages (size * msg_per_seq)
                        let mut q = factory();
                        for i in 0..total_messages {
                            let _ = q.push(generator(i, num_seqs.1));
                        }
                        q
                    },
                    |mut q| {
                        let mut cnt = 0usize;
                        while let Some(_m) = q.pop() {
                            cnt += 1;
                            // only advance_seq if the current seq bucket is empty
                            if q.is_empty() {
                                q.advance_seq();
                            }
                        }
                        debug_assert_eq!(cnt, total_messages);
                    },
                    criterion::BatchSize::LargeInput,
                )
            });
        }
    }

    group.finish();
}

fn bench_peek<F, G, T, M>(
    c: &mut Criterion,
    id: &str,
    factory: F,
    generator: G,
    sizes: &[usize],
    msg_per_seq: &[usize],
) where
    F: Fn() -> T + Send + Sync + 'static,
    G: Fn(usize, usize) -> M + Send + Sync + 'static,
    T: TTboQueue<M> + Send + 'static,
    M: Orderable + Clone + Send + 'static,
{
    let mut group = c.benchmark_group(format!("tbo_peek_{id}"));

    for &size in sizes {
        for &msg_per_seq in msg_per_seq {
            let total_messages = size * msg_per_seq;

            group.throughput(Throughput::Elements(total_messages as u64));
            let seq = BenchSeq(size, msg_per_seq);
            group.bench_with_input(BenchmarkId::from_parameter(seq), &seq, |b, &num_seqs| {
                b.iter_batched(
                    || {
                        let mut q = factory();
                        for i in 0..total_messages {
                            let _ = q.push(generator(i, num_seqs.1));
                        }
                        q
                    },
                    |mut q| {
                        // repeatedly peek and advance the sequence so peek returns None eventually
                        while let Some(_m) = q.peek() {
                            let _ = q.peek();
                            q.pop();
                            // only advance_seq if the current seq bucket is empty
                            if q.is_empty() {
                                q.advance_seq();
                            }
                        }
                    },
                    criterion::BatchSize::LargeInput,
                )
            });
        }
    }

    group.finish();
}

fn bench_advance_install_clear<F, G, T, M>(
    c: &mut Criterion,
    id: &str,
    factory: F,
    generator: G,
    msg_per_seq: usize,
) where
    F: Fn() -> T + Send + Sync + 'static,
    G: Fn(usize, usize) -> M + Send + Sync + 'static,
    T: TTboQueue<M> + Send + 'static,
    M: Orderable + Clone + Send + 'static,
{
    let mut group = c.benchmark_group(format!("tbo_misc_{id}"));

    group.bench_function("advance_seq_many", |b| {
        b.iter_batched(
            || {
                let mut q = factory();
                for i in 0..1000usize {
                    let _ = q.push(generator(i, msg_per_seq));
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
                    let _ = q.push(generator(i, msg_per_seq));
                }
                q
            },
            |mut q| {
                q.advance_to_seq(SeqNo::from(500u32))
                    .expect("install_seq should succeed");
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.bench_function("clear_queue", |b| {
        b.iter_batched(
            || {
                let mut q = factory();
                for i in 0..10000usize {
                    let _ = q.push(generator(i, msg_per_seq));
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
// Here, sizes represent the number of sequence numbers, and we vary the messages per sequence
fn tbo_queue_bench<Q>(c: &mut Criterion, name: &str)
where
    Q: TTboQueue<BenchMsg> + Send + 'static,
{
    // Test configurations: number of sequence numbers to generate
    let seq_sizes = [1usize, 10usize, 100usize, 1000usize];

    let msg_per_seq = [10usize, 5usize];

    let factory = || Q::default();

    let generator =
        |i: usize, msg_per_seq: usize| BenchMsg::new(SeqNo::from((i / msg_per_seq) as u32));

    bench_push(c, name, factory, generator, &seq_sizes, &msg_per_seq);
    bench_pop(c, name, factory, generator, &seq_sizes, &msg_per_seq);
    bench_peek(c, name, factory, generator, &seq_sizes, &msg_per_seq);

    for &msg_per_seq in &msg_per_seq {
        bench_advance_install_clear(c, name, factory, generator, msg_per_seq);
    }
}

fn tbo_queue_vec_bench(c: &mut Criterion) {
    tbo_queue_bench::<VTboQueue<BenchMsg>>(c, "vec_tbo_queue");
}

fn tbo_tree_queue_bench(c: &mut Criterion) {
    tbo_queue_bench::<TboQueue<BenchMsg>>(c, "btree_tbo_queue");
}

criterion_group!(benches, tbo_tree_queue_bench, tbo_queue_vec_bench);
criterion_main!(benches);
