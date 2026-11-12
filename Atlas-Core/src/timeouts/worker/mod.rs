use getset::{CopyGetters, Getters};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Add;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, SystemTimeError};

use crate::timeouts::{
    Timeout, TimeoutAck, TimeoutIdentification, TimeoutRequest, TimeoutWorkerResponder,
};
use atlas_common::channel::TryRecvError;
use atlas_common::channel::sync::ChannelSyncRx;
use atlas_common::collections::HashMap;
use atlas_common::node_id::NodeId;
use atlas_common::quiet_opt_unwrap;
use thiserror::Error;
use tracing::error;

/// How long an ack that arrives before its timeout is registered is kept.
const EARLY_ACK_TTL: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum WorkerMessage {
    Request(TimeoutRequest),
    Requests(Vec<TimeoutRequest>),
    Ack(TimeoutAck),
    Acks(Vec<TimeoutAck>),
    Cancel(TimeoutIdentification),
    CancelMultiple(Vec<TimeoutIdentification>),
    CancelAll(Arc<str>),
    ResetAll(Arc<str>),
}

#[allow(dead_code)]
enum TimeoutPhase {
    // This request has never timed out
    NeverTimedOut,
    TimedOut(usize, SystemTime),
}

#[derive(Getters, CopyGetters)]
struct RegisteredTimeout {
    #[get_copy]
    timeout_time: u64,
    #[get]
    timeout_phase: TimeoutPhase,
    #[get]
    #[allow(dead_code)]
    time_made: SystemTime,
    #[get]
    acks_received: BTreeSet<NodeId>,
    #[get]
    request: TimeoutRequest,
}

pub struct TimeoutWorker<WR> {
    #[allow(dead_code)]
    our_node_id: NodeId,
    #[allow(dead_code)]
    worker_id: u32,
    default_timeout_duration: Duration,

    work_rx_channel: ChannelSyncRx<WorkerMessage>,

    // Requests which can receive multiple timeouts are tracked in
    // this map
    watched_requests: HashMap<TimeoutIdentification, Rc<RefCell<RegisteredTimeout>>>,

    // The timeouts we are currently waiting for
    pending_timeout_heap:
        BTreeMap<u64, HashMap<TimeoutIdentification, Rc<RefCell<RegisteredTimeout>>>>,
    // The notifier to send timeouts to
    timeout_notifier: WR,

    // Acks that arrived before their timeout was registered, with when the first one did.
    early_acks: HashMap<TimeoutIdentification, BTreeSet<NodeId>>,
    early_ack_order: VecDeque<(u64, TimeoutIdentification)>,
    early_ack_ttl: Duration,
}

pub(super) fn initialize_worker_thread<WR>(
    our_id: NodeId,
    worker_id: usize,
    work_receiver: ChannelSyncRx<WorkerMessage>,
    notifier: WR,
) where
    WR: TimeoutWorkerResponder + 'static,
{
    std::thread::Builder::new()
        .name(format!("TimeoutWorker-{worker_id}"))
        .spawn(move || {
            let mut worker = TimeoutWorker {
                our_node_id: our_id,
                worker_id: u32::try_from(worker_id)
                    .expect("Failed to convert worker id to unsigned integer"),
                default_timeout_duration: Duration::default(),
                work_rx_channel: work_receiver,
                watched_requests: HashMap::default(),
                pending_timeout_heap: BTreeMap::default(),
                timeout_notifier: notifier,
                early_acks: HashMap::default(),
                early_ack_order: VecDeque::default(),
                early_ack_ttl: EARLY_ACK_TTL,
            };

            loop {
                if let Err(err) = worker.run() {
                    error!("Timeout worker error: {err:?}");
                }
            }
        })
        .expect("Failed to spawn timeout worker thread");
}

impl<WR> TimeoutWorker<WR>
where
    WR: TimeoutWorkerResponder,
{
    pub fn new(
        our_id: NodeId,
        worker_id: u32,
        default_timeout_duration: Duration,
        work_rx_channel: ChannelSyncRx<WorkerMessage>,
        timeout_notifier: WR,
    ) -> Self {
        Self {
            our_node_id: our_id,
            worker_id,
            default_timeout_duration,
            work_rx_channel,
            watched_requests: Default::default(),
            pending_timeout_heap: Default::default(),
            timeout_notifier,
            early_acks: Default::default(),
            early_ack_order: Default::default(),
            early_ack_ttl: EARLY_ACK_TTL,
        }
    }

    fn run(&mut self) -> Result<(), TimeoutError> {
        let duration = Duration::from_millis(1000);

        loop {
            match self.work_rx_channel.recv_timeout(duration) {
                Ok(message) => {
                    self.process_message(message)?;
                }
                Err(e) => {
                    if let TryRecvError::Timeout { .. } = e {
                    } else {
                        error!("Error receiving message: {:?}", e);
                    }
                }
            }

            self.process_timeouts()?;
        }
    }

    //#[instrument(skip(self), level = Level::DEBUG)]
    fn process_message(
        &mut self,
        message: WorkerMessage,
    ) -> Result<(), ProcessTimeoutMessageError> {
        match message {
            WorkerMessage::Request(request) => {
                self.handle_timeout_request(request)?;
            }
            WorkerMessage::Requests(requests) => {
                requests
                    .into_iter()
                    .try_for_each(|rq| self.handle_timeout_request(rq))?;
            }
            WorkerMessage::Ack(ack) => {
                self.handle_timeout_ack(ack)?;
            }
            WorkerMessage::Acks(acks) => {
                acks.into_iter()
                    .try_for_each(|ack| self.handle_timeout_ack(ack))?;
            }
            WorkerMessage::Cancel(timeout_id) => {
                self.remove_time_out(&timeout_id);
            }
            WorkerMessage::CancelMultiple(timeout_ids) => {
                timeout_ids
                    .into_iter()
                    .for_each(|id| self.remove_time_out(&id));
            }
            WorkerMessage::CancelAll(module_name) => {
                self.cancel_all_rqs_for_mod(module_name)?;
            }
            WorkerMessage::ResetAll(mod_name) => {
                self.reset_all_rqs_for_mod(mod_name)?;
            }
        }

        Ok(())
    }

    pub fn handle_timeout_request(
        &mut self,
        timeout_request: TimeoutRequest,
    ) -> Result<(), ProcessTimeoutError> {
        let registered_rq = Rc::new(RefCell::new(RegisteredTimeout::new(timeout_request)?));

        let request_id = registered_rq.borrow().request().id().clone();

        if let Some(acks) = self.early_acks.remove(&request_id) {
            let mut rq = registered_rq.borrow_mut();

            rq.acks_received.extend(acks);

            if rq.acks_received.len() >= rq.request().needed_acks() {
                return Ok(());
            }
        }

        self.watched_requests
            .entry(request_id.clone())
            .or_insert(registered_rq.clone());

        let timeout_time = registered_rq.borrow().timeout_time();

        self.pending_timeout_heap
            .entry(timeout_time)
            .or_default()
            .insert(request_id, registered_rq);

        Ok(())
    }

    fn re_register_timeout(
        &mut self,
        timeout: Rc<RefCell<RegisteredTimeout>>,
    ) -> Result<(), ProcessTimeoutError> {
        let timeout_time = SystemTime::now()
            .add(*timeout.borrow().request().duration())
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_millis() as u64;

        timeout.borrow_mut().timeout_time = timeout_time;

        let timeout_id = timeout.borrow().request().id().clone();

        self.pending_timeout_heap
            .entry(timeout_time)
            .or_default()
            .insert(timeout_id, timeout);

        Ok(())
    }

    pub fn handle_timeout_ack(&mut self, timeout_ack: TimeoutAck) -> Result<(), AcceptAckError> {
        let should_remove = if let Some(watched) = self.watched_requests.get(timeout_ack.id()) {
            let mut mut_guard = watched.borrow_mut();

            mut_guard.register_ack(timeout_ack.from())?;

            mut_guard.acks_received().len() >= mut_guard.request().needed_acks()
        } else {
            self.record_early_ack(timeout_ack.id().clone(), timeout_ack.from())?;

            false
        };

        if should_remove {
            self.remove_time_out(timeout_ack.id());
        }

        Ok(())
    }

    fn record_early_ack(
        &mut self,
        id: TimeoutIdentification,
        from: NodeId,
    ) -> Result<(), AcceptAckError> {
        let now = now_millis()?;

        match self.early_acks.get_mut(&id) {
            Some(acks) => {
                acks.insert(from);
            }
            None => {
                self.early_ack_order.push_back((now, id.clone()));
                self.early_acks.insert(id, BTreeSet::from([from]));
            }
        }

        Ok(())
    }

    fn purge_early_acks(&mut self, now: u64) {
        let ttl = self.early_ack_ttl.as_millis() as u64;

        while let Some((at, _)) = self.early_ack_order.front() {
            if at + ttl > now {
                break;
            }

            if let Some((_, id)) = self.early_ack_order.pop_front() {
                self.early_acks.remove(&id);
            }
        }
    }

    //#[instrument(skip(self), level = Level::DEBUG)]
    fn remove_time_out(&mut self, timeout_id: &TimeoutIdentification) {
        if let Some(watched) = self.watched_requests.remove(timeout_id) {
            let timeout_time = watched.borrow().timeout_time();

            if let Some(timeouts) = self.pending_timeout_heap.get_mut(&timeout_time) {
                timeouts.remove(timeout_id);
            } else {
                error!(
                    "Removed timeout that was no longer in the pending timeout heap? {:?}",
                    timeout_id
                );
            }
        } else {
            error!(
                "Provided timeout that is not currently being tracked {:?}",
                timeout_id
            );
        }
    }

    //#[instrument(skip(self), level = Level::DEBUG)]
    fn process_timeouts(&mut self) -> Result<(), TimeoutError> {
        let current_sys_time = SystemTime::now();

        let now = current_sys_time
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_millis() as u64;

        self.purge_early_acks(now);

        let mut timeouts = Vec::new();

        while let Some((timeout, _)) = self.pending_timeout_heap.first_key_value() {
            if *timeout > now {
                break;
            }

            let (_, requests) = quiet_opt_unwrap!(
                self.pending_timeout_heap.pop_first(),
                Err(TimeoutError::MapPopFailed)
            );

            requests
                .iter()
                .for_each(|(_rq_id, rq)| rq.borrow_mut().timed_out());

            let (cumulative_stream, non_cumulative_stream): (Vec<_>, Vec<_>) = requests
                .iter()
                .partition(|(_rq_id, rq)| rq.borrow().request().is_cumulative());

            // Re register the cumulative timeouts for the next timeout
            cumulative_stream
                .into_iter()
                .try_for_each(|(_rq_id, rq)| self.re_register_timeout(rq.clone()))?;

            // Remove the non-cumulative ones from the watched list
            non_cumulative_stream.into_iter().for_each(|(rq_id, _rq)| {
                self.watched_requests.remove(rq_id);
            });

            let mut partial_timeouts = requests
                .into_values()
                .map(|rq| {
                    let rq_borrow = rq.borrow();

                    let extra_info = rq_borrow
                        .request()
                        .extra_info
                        .as_ref()
                        .map(|ex| dyn_clone::clone_box(&**ex));

                    Timeout {
                        id: rq_borrow.request().id().clone(),
                        timeout_count: rq_borrow.timeout_phase().timeout_count(),
                        timeout_time: current_sys_time,
                        extra_info,
                    }
                })
                .collect::<Vec<_>>();

            timeouts.append(&mut partial_timeouts);
        }

        if !timeouts.is_empty() {
            self.timeout_notifier.report_timeouts(timeouts)?;
        }

        Ok(())
    }

    fn cancel_all_rqs_for_mod(
        &mut self,
        mod_name: Arc<str>,
    ) -> Result<(), ClearAllOcurrencesError> {
        self.watched_requests
            .retain(|k, _v| !cmp_mod_name(&mod_name, k.mod_id()));

        self.pending_timeout_heap.iter_mut().for_each(|(_, v)| {
            v.retain(|_k, rq| !cmp_mod_name(rq.borrow().request().id().mod_id(), &mod_name))
        });

        Ok(())
    }

    fn reset_all_rqs_for_mod(&mut self, mod_name: Arc<str>) -> Result<(), ResetTimeoutError> {
        let mut removed_timeouts: Vec<_> = self
            .pending_timeout_heap
            .values_mut()
            .flat_map(|timeouts| {
                timeouts.extract_if(|_rq_id, rq| {
                    cmp_mod_name(rq.borrow().request().id().mod_id(), &mod_name)
                })
            })
            .collect();

        removed_timeouts.iter_mut().for_each(|(_timeout_id, rq)| {
            let mut timeout_ref = rq.borrow_mut();

            timeout_ref.timeout_phase = TimeoutPhase::NeverTimedOut;
            timeout_ref.acks_received.clear();
        });

        let timeout_time = SystemTime::now()
            .add(self.default_timeout_duration)
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_millis() as u64;

        let pending_timeout_map = self.pending_timeout_heap.entry(timeout_time).or_default();

        removed_timeouts.into_iter().for_each(|(timeout_id, rq)| {
            pending_timeout_map.insert(timeout_id, rq);
        });

        Ok(())
    }
}

impl TimeoutPhase {
    fn next_timeout(&self) -> Self {
        match self {
            Self::NeverTimedOut => Self::TimedOut(1, SystemTime::now()),
            Self::TimedOut(n, _) => Self::TimedOut(*n + 1, SystemTime::now()),
        }
    }

    fn timeout_count(&self) -> usize {
        match self {
            Self::NeverTimedOut => 0,
            Self::TimedOut(n, _) => *n,
        }
    }

    #[allow(dead_code)]
    fn last_timeout_time(&self) -> Option<SystemTime> {
        match self {
            Self::NeverTimedOut => None,
            Self::TimedOut(_, t) => Some(*t),
        }
    }
}

impl RegisteredTimeout {
    fn new(request: TimeoutRequest) -> Result<Self, SystemTimeError> {
        let end_timeout_time = SystemTime::now() + request.duration;

        let timeout_int_time = end_timeout_time
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_millis() as u64;

        Ok(Self {
            timeout_time: timeout_int_time,
            timeout_phase: TimeoutPhase::NeverTimedOut,
            time_made: SystemTime::now(),
            acks_received: BTreeSet::new(),
            request,
        })
    }

    fn register_ack(&mut self, from: NodeId) -> Result<(), AcceptAckError> {
        if !self.acks_received.insert(from) {
            return Err(AcceptAckError::NodeAlreadyAcked(from));
        }

        Ok(())
    }

    fn timed_out(&mut self) {
        self.timeout_phase = self.timeout_phase.next_timeout();
    }
}

fn now_millis() -> Result<u64, SystemTimeError> {
    Ok(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_millis() as u64)
}

fn cmp_mod_name(mod_name_1: impl AsRef<str>, mod_name_2: impl AsRef<str>) -> bool {
    mod_name_1.as_ref() == mod_name_2.as_ref()
}

#[derive(Error, Debug)]
pub enum ProcessTimeoutMessageError {
    #[error("Ack process failed: {0}")]
    AckProcessFailed(#[from] AcceptAckError),
    #[error("Failed to process timeout: {0}")]
    TimeoutProcessFailed(#[from] ProcessTimeoutError),
    #[error("Failed to clear all occurrences {0}")]
    ClearingAllOccurrences(#[from] ClearAllOcurrencesError),
    #[error("Failed to reset timeouts: {0}")]
    ResetTimeouts(#[from] ResetTimeoutError),
}

#[derive(Error, Debug)]
pub enum ClearAllOcurrencesError {}

#[derive(Error, Debug)]
pub enum ResetTimeoutError {
    #[error("Failed to calculate system time? {0}")]
    SystemTime(#[from] SystemTimeError),
}

#[derive(Error, Debug)]
pub enum AcceptAckError {
    #[error("Node already ACKed {0:?}")]
    NodeAlreadyAcked(NodeId),
    #[error("Failed to calculate system time? {0}")]
    SystemTime(#[from] SystemTimeError),
}

#[derive(Error, Debug)]
pub enum ProcessTimeoutError {
    #[error("Failed to calculate system time? {0}")]
    SystemTimeError(#[from] SystemTimeError),
}

#[derive(Error, Debug)]
pub enum TimeoutError {
    #[error("Failed to calculate system time? {0}")]
    SystemTime(#[from] SystemTimeError),
    #[error("Re-Register cumulative timeout error {0}")]
    RegisterTimeout(#[from] ProcessTimeoutError),
    #[error("Failed to process message timeouts {0}")]
    MessageProcess(#[from] ProcessTimeoutMessageError),
    #[error("Failed to notify of timeouts {0}")]
    Notifier(#[from] anyhow::Error),
    #[error("Failed to obtain timeouts from map")]
    MapPopFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeouts::TimeoutID;
    use atlas_common::channel::sync::new_bounded_sync;
    use atlas_common::ordering::SeqNo;

    #[derive(Clone)]
    struct Discard;

    impl TimeoutWorkerResponder for Discard {
        fn report_timeouts(&self, _: Vec<Timeout>) -> atlas_common::error::Result<()> {
            Ok(())
        }
    }

    fn worker() -> TimeoutWorker<Discard> {
        let (_tx, rx) = new_bounded_sync(8, Some("test"));

        TimeoutWorker::new(NodeId(0), 0, Duration::from_secs(1), rx, Discard)
    }

    fn id() -> TimeoutIdentification {
        TimeoutIdentification {
            mod_id: Arc::from("TestMod"),
            timeout_id: TimeoutID::SessionBased {
                session: SeqNo::ZERO,
                seq_no: SeqNo::ZERO,
                from: NodeId(7),
            },
        }
    }

    fn request(needed_acks: usize) -> TimeoutRequest {
        TimeoutRequest {
            id: id(),
            duration: Duration::from_secs(1),
            needed_acks,
            is_cumulative: true,
            extra_info: None,
        }
    }

    fn ack(from: u32) -> TimeoutAck {
        TimeoutAck {
            id: id(),
            from: NodeId(from),
        }
    }

    fn pending(w: &TimeoutWorker<Discard>) -> usize {
        w.pending_timeout_heap.values().map(|m| m.len()).sum()
    }

    #[test]
    fn an_ack_that_arrives_first_completes_the_timeout() {
        let mut w = worker();

        w.handle_timeout_ack(ack(1)).unwrap();
        w.handle_timeout_request(request(1)).unwrap();

        assert_eq!(pending(&w), 0);
        assert!(w.watched_requests.is_empty());
        assert!(w.early_acks.is_empty());
    }

    #[test]
    fn an_early_ack_below_the_needed_count_still_counts_towards_it() {
        let mut w = worker();

        w.handle_timeout_ack(ack(1)).unwrap();
        w.handle_timeout_request(request(2)).unwrap();
        assert_eq!(pending(&w), 1);

        w.handle_timeout_ack(ack(2)).unwrap();
        assert_eq!(pending(&w), 0);
    }

    #[test]
    fn early_acks_expire_and_do_not_complete_a_later_timeout() {
        let mut w = worker();

        w.handle_timeout_ack(ack(1)).unwrap();

        let ttl = w.early_ack_ttl.as_millis() as u64;
        w.purge_early_acks(now_millis().unwrap() + ttl + 1);

        assert!(w.early_acks.is_empty() && w.early_ack_order.is_empty());

        w.handle_timeout_request(request(1)).unwrap();
        assert_eq!(pending(&w), 1);
    }
}
