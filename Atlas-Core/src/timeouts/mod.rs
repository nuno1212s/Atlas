use anyhow::Context;
use dyn_clone::DynClone;
use getset::{CopyGetters, Getters};
use itertools::Itertools;
use std::any::Any;
use std::cmp::Ordering;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use atlas_common::channel::sync::{new_bounded_sync, ChannelSyncTx};
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;

use crate::request_pre_processing::operation_key_raw;
use crate::timeouts::timeout::{TimeoutModHandle, TimeoutableMod};
use crate::timeouts::worker::WorkerMessage;

pub mod tests;
pub mod timeout;
pub mod worker;

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub enum TimeoutID {
    SeqNoBased(SeqNo),
    SessionBased {
        session: SeqNo,
        seq_no: SeqNo,
        from: NodeId,
    },
}

#[derive(Hash, Ord, Eq, PartialOrd, PartialEq, Clone, Debug, Getters, CopyGetters)]
pub struct TimeoutIdentification {
    #[get = "pub"]
    mod_id: Arc<str>,
    #[get = "pub"]
    timeout_id: TimeoutID,
}

/// A time-out that has occurred
#[derive(Debug, CopyGetters, Getters)]
pub struct Timeout {
    #[get = "pub"]
    id: TimeoutIdentification,
    #[get_copy = "pub"]
    timeout_count: usize,
    #[get_copy = "pub"]
    timeout_time: SystemTime,
    extra_info: Option<Box<dyn TimeOutable>>,
}

#[derive(CopyGetters, Getters, Debug)]
pub struct TimeoutRequest {
    #[get]
    id: TimeoutIdentification,
    #[get]
    duration: Duration,
    #[get_copy]
    needed_acks: usize,
    #[get_copy]
    is_cumulative: bool,
    extra_info: Option<Box<dyn TimeOutable>>,
}

/// A timeout trait, representing the behaviour needed from a timeout request
pub trait TimeOutable: DynClone + Debug + Send {
    /// Turn a box of this object into a box of dyn Any, so it can be cast
    fn into_any(self: Box<Self>) -> Box<dyn Any>;

    /// casting to any trait object
    fn as_any(&self) -> &dyn Any;
}

#[derive(Getters, CopyGetters, Debug)]
pub struct TimeoutAck {
    #[get = "pub"]
    id: TimeoutIdentification,
    #[get_copy]
    from: NodeId,
}

#[derive(Clone)]
pub struct TimeoutsHandle {
    worker_handles: Vec<ChannelSyncTx<WorkerMessage>>,
}

pub fn initialize_timeouts<WR>(
    our_id: NodeId,
    num_workers: usize,
    channel_size: usize,
    timeout_output: WR,
) -> TimeoutsHandle
where
    WR: TimeoutWorkerResponder + 'static,
{
    let mut handles = Vec::with_capacity(num_workers);

    for worker in 0..num_workers {
        let (tx, rx) =
            new_bounded_sync(channel_size, Some(format!("TimeoutWorker Thread {worker}")));

        worker::initialize_worker_thread(our_id, worker, rx, timeout_output.clone());

        handles.push(tx)
    }

    TimeoutsHandle {
        worker_handles: handles,
    }
}

impl TimeoutsHandle {
    fn worker_for_timeout(
        &self,
        timeout_id: &TimeoutIdentification,
    ) -> &ChannelSyncTx<WorkerMessage> {
        &self.worker_handles[self.worker_id_for_timeout(timeout_id)]
    }

    fn worker_id_for_timeout(&self, timeout_id: &TimeoutIdentification) -> usize {
        match timeout_id.timeout_id {
            TimeoutID::SeqNoBased(_) => 0,
            TimeoutID::SessionBased { session, from, .. } => {
                operation_key_raw(from, session) as usize % self.worker_handles.len()
            }
        }
    }

    pub fn gen_mod_handle_with_name(&self, mod_name: Arc<str>) -> TimeoutModHandle {
        TimeoutModHandle::from_name(mod_name, self.clone())
    }

    pub fn gen_mod_handle_for<M, R>(&self) -> TimeoutModHandle
    where
        M: TimeoutableMod<R>,
    {
        TimeoutModHandle::from_timeout_mod::<M, R>(self.clone())
    }

    //#[instrument(skip(self), level = "trace")]
    pub fn request_timeout(
        &self,
        timeout_id: TimeoutIdentification,
        extra_info: Option<Box<dyn TimeOutable>>,
        duration: Duration,
        needed_acks: usize,
        cumulative: bool,
    ) -> atlas_common::error::Result<()> {
        self.worker_for_timeout(&timeout_id)
            .send(WorkerMessage::Request(TimeoutRequest {
                id: timeout_id,
                duration,
                needed_acks,
                is_cumulative: cumulative,
                extra_info,
            }))
            .context("Failed to send message to timeout worker handle")
    }

    //#[instrument(skip(self, timeout_id), level = "trace")]
    pub fn request_timeouts(
        &self,
        timeout_id: Vec<(TimeoutIdentification, Option<Box<dyn TimeOutable>>)>,
        duration: Duration,
        needed_acks: usize,
        cumulative: bool,
    ) -> atlas_common::error::Result<()> {
        timeout_id
            .into_iter()
            .map(|(id, extra_info)| TimeoutRequest {
                id,
                duration,
                needed_acks,
                is_cumulative: cumulative,
                extra_info,
            })
            .chunk_by(|rq| self.worker_id_for_timeout(rq.id()))
            .into_iter()
            .try_for_each(|(worker_id, group)| {
                self.worker_handles[worker_id].send(WorkerMessage::Requests(group.collect()))
            })
            .context("Failed to send message to timeout worker handle")
    }

    //#[instrument(skip(self), level = "trace")]
    pub fn ack_received(
        &self,
        timeout_id: TimeoutIdentification,
        from: NodeId,
    ) -> atlas_common::error::Result<()> {
        self.worker_for_timeout(&timeout_id)
            .send(WorkerMessage::Ack(TimeoutAck {
                id: timeout_id,
                from,
            }))
            .context("Failed to send message to timeout worker handle")
    }

    //#[instrument(skip(self, acks), level = "trace", fields(acks = acks.len()))]
    pub fn acks_received(
        &self,
        acks: Vec<(TimeoutIdentification, NodeId)>,
    ) -> atlas_common::error::Result<()> {
        acks.into_iter()
            .map(|(id, from)| TimeoutAck { id, from })
            .chunk_by(|ack| self.worker_id_for_timeout(ack.id()))
            .into_iter()
            .try_for_each(|(worker_id, group)| {
                self.worker_handles[worker_id].send(WorkerMessage::Acks(group.collect()))
            })
            .context("Failed to send message to timeout worker handle")
    }

    //#[instrument(skip(self), level = "trace")]
    pub fn cancel_timeout(
        &self,
        timeout: TimeoutIdentification,
    ) -> atlas_common::error::Result<()> {
        self.worker_for_timeout(&timeout)
            .send(WorkerMessage::Cancel(timeout))
            .context("Failed to send message to timeout worker handle")
    }

    //#[instrument(skip(self), level = "trace", fields(cancelled_timeouts = timeouts.len()))]
    pub fn cancel_timeouts(
        &self,
        timeouts: Vec<TimeoutIdentification>,
    ) -> atlas_common::error::Result<()> {
        timeouts
            .into_iter()
            .chunk_by(|timeout| self.worker_id_for_timeout(timeout))
            .into_iter()
            .try_for_each(|(worker_id, group)| {
                self.worker_handles[worker_id].send(WorkerMessage::CancelMultiple(group.collect()))
            })
            .context("Failed to send message to timeout worker handle")
    }

    //#[instrument(skip(self), level = "trace")]
    pub fn cancel_all_timeouts_for_mod(
        &self,
        mod_name: Arc<str>,
    ) -> atlas_common::error::Result<()> {
        self.worker_handles
            .iter()
            .try_for_each(|worker| worker.send(WorkerMessage::CancelAll(mod_name.clone())))
            .context("Failed to send message to timeout worker handle")
    }

    pub fn reset_all_timeouts_for_mod(
        &self,
        mod_name: Arc<str>,
    ) -> atlas_common::error::Result<()> {
        self.worker_handles
            .iter()
            .try_for_each(|worker| worker.send(WorkerMessage::ResetAll(mod_name.clone())))
            .context("Failed to send message to timeout worker handle")
    }
}

impl PartialOrd for TimeoutID {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimeoutID {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (TimeoutID::SeqNoBased(a), TimeoutID::SeqNoBased(b)) => a.cmp(b),
            (
                TimeoutID::SessionBased {
                    session: session_1,
                    seq_no: seq_1,
                    from: from_1,
                },
                TimeoutID::SessionBased {
                    session: session_2,
                    seq_no: seq_2,
                    from: from_2,
                },
            ) => match from_1.cmp(from_2) {
                Ordering::Equal => match session_1.cmp(session_2) {
                    Ordering::Equal => seq_1.cmp(seq_2),
                    other => other,
                },
                other => other,
            },
            (TimeoutID::SeqNoBased(_), TimeoutID::SessionBased { .. }) => Ordering::Less,
            (TimeoutID::SessionBased { .. }, TimeoutID::SeqNoBased(_)) => Ordering::Greater,
        }
    }
}

/// The trait detailing the worker behaviour we require
pub trait TimeoutWorkerResponder: Send + Clone {
    /// Report a timeout to the rest of the system
    fn report_timeouts(&self, timeouts: Vec<Timeout>) -> atlas_common::error::Result<()>;
}

impl TimeoutRequest {
    pub fn new(
        id: TimeoutIdentification,
        duration: Duration,
        needed_acks: usize,
        is_cumulative: bool,
        extra_info: Option<Box<dyn TimeOutable>>,
    ) -> Self {
        Self {
            id,
            duration,
            needed_acks,
            is_cumulative,
            extra_info,
        }
    }

    pub fn extra_info(&self) -> Option<&dyn TimeOutable> {
        self.extra_info.as_deref()
    }
}

impl Timeout {
    pub fn extra_info(&self) -> Option<&dyn TimeOutable> {
        self.extra_info.as_deref()
    }
}

impl TimeoutIdentification {
    pub fn new(mod_name: Arc<str>, seq: SeqNo) -> Self {
        Self {
            mod_id: mod_name,
            timeout_id: TimeoutID::SeqNoBased(seq),
        }
    }

    pub fn new_session_based(mod_name: Arc<str>, session: SeqNo, seq: SeqNo, from: NodeId) -> Self {
        Self {
            mod_id: mod_name,
            timeout_id: TimeoutID::SessionBased {
                session,
                seq_no: seq,
                from,
            },
        }
    }

    pub fn new_from_id(mod_name: Arc<str>, id: TimeoutID) -> Self {
        Self {
            mod_id: mod_name,
            timeout_id: id,
        }
    }
}

impl TimeoutAck {
    pub fn new(id: TimeoutIdentification, from: NodeId) -> Self {
        Self { id, from }
    }
}
