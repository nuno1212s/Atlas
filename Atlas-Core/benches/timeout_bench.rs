use atlas_common::channel;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_core::timeouts;
use atlas_core::timeouts::worker::TimeoutWorker;
use atlas_core::timeouts::{
    TimeoutAck, TimeoutIdentification, TimeoutRequest, TimeoutWorkerResponder,
};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

static MOD_NAME: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("TestMod"));
static DEFAULT_TIMEOUT: LazyLock<Duration> = LazyLock::new(|| Duration::from_secs(1));

const NEEDED_ACKS: usize = 3;

#[derive(Clone)]
struct MockWR;

impl TimeoutWorkerResponder for MockWR {
    fn report_timeouts(&self, _timeouts: Vec<timeouts::Timeout>) -> Result<()> {
        Ok(())
    }
}

#[divan::bench(args= [1000, 10000, 100_000])]
fn benchmark_timeout(bencher: divan::Bencher, requests: u32) {
    let (_c, channel) = channel::sync::new_bounded_sync(1, Some("TimeoutWorker"));

    let mut worker = TimeoutWorker::new(NodeId(0), 0, *DEFAULT_TIMEOUT, channel, MockWR);

    let rqs: Vec<_> = (0..requests)
        .map(|rq| {
            TimeoutIdentification::new_session_based(
                MOD_NAME.clone(),
                SeqNo::from(rq),
                SeqNo::ZERO,
                NodeId(0),
            )
        })
        .collect();

    bencher.bench_local(|| {
        rqs.iter()
            .map(|timeout_id| {
                TimeoutRequest::new(
                    timeout_id.clone(),
                    *DEFAULT_TIMEOUT,
                    NEEDED_ACKS,
                    true,
                    None,
                )
            })
            .for_each(|rqs| {
                worker
                    .handle_timeout_request(rqs)
                    .expect("Failed to handle timeout request");
            });

        for timeout_id in &rqs {
            (0..NEEDED_ACKS).for_each(|id| {
                let ack = TimeoutAck::new(timeout_id.clone(), NodeId::from(id));

                worker
                    .handle_timeout_ack(ack)
                    .expect("Failed to ack timeout");
            });
        }
    });
}

fn main() {
    divan::main();
}
