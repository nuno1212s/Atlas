use std::ops::{Deref, DerefMut};
use std::time::{Duration, Instant};
use std::vec::IntoIter;

use atlas_common::channel::mixed::ChannelMixedRx;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::channel::{RecvError, TryRecvError};
use atlas_common::error::Result;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::{Header, StoredMessage};
use atlas_metrics::metrics::metric_duration;

use crate::messages::{ClientRqInfo, ForwardedRequestsMessage, SessionBased};
use crate::metric::RQ_PP_WORKER_PROPOSER_PASSING_TIME_ID;
use crate::timeouts::timeout::ModTimeout;

pub mod network;
pub mod work_dividers;

/// The work partitioner is responsible for deciding which worker should process a given request
/// This should sign a contract to maintain all client sessions in the same worker, never changing
/// A session is defined by the client ID and the session ID.
///
pub trait WorkPartitioner: Send {
    /// Get the worker that should process this request
    fn get_worker_for<O>(rq_info: &Header, message: &O, worker_count: usize) -> usize
    where
        O: SessionBased;

    /// Get the worker that should process this request
    fn get_worker_for_processed(rq_info: &ClientRqInfo, worker_count: usize) -> usize;

    fn get_worker_for_raw(from: NodeId, session: SeqNo, worker_count: usize) -> usize;
}

pub trait RequestPProcessorAsync<O> {
    fn clone_pending_rqs(
        &self,
        client_rqs: Vec<ClientRqInfo>,
    ) -> Result<ChannelMixedRx<Vec<StoredMessage<O>>>>;

    fn collect_pending_rqs(&self) -> Result<ChannelMixedRx<Vec<StoredMessage<O>>>>;
}

pub trait RequestPProcessorSync<O> {
    /// Clone the given pending client requests
    fn clone_pending_rqs(&self, client_rqs: Vec<ClientRqInfo>) -> Result<Vec<StoredMessage<O>>>;

    /// Collect all pending client requests
    fn collect_pending_rqs(&self) -> Result<Vec<StoredMessage<O>>>;
}

/// The request pre-processor timeout trait.
pub trait RequestPreProcessorTimeout {
    /// Process a given message containing timeouts
    fn process_timeouts(
        &self,
        timeouts: Vec<ModTimeout>,
        response_channel: ChannelSyncTx<(Vec<ModTimeout>, Vec<ModTimeout>)>,
    ) -> Result<()>;
}

/// The request pre-processor trait.
///
/// This trait is responsible for processing requests that have been forwarded to the current replica.
pub trait RequestPreProcessing<O> {
    /// Process a given message containing forwarded requests
    fn process_forwarded_requests(
        &self,
        message: StoredMessage<ForwardedRequestsMessage<O>>,
    ) -> Result<()>;

    /// Process a given message containing stopped requests
    fn process_stopped_requests(&self, messages: Vec<StoredMessage<O>>) -> Result<()>;

    /// Process a batch of requests that have been ordered
    fn process_decided_batch(&self, client_rqs: Vec<ClientRqInfo>) -> Result<()>;
}

/// The trait that deligns the behaviour necessary for the request pre processing to handle
/// client requests.
///
/// Specifically this is made in order to handle clients that are connecting to us,
/// and therefore are starting from a blank slate in terms of session and operation sequence numbers
pub trait RequestClientPreProcessing {
    fn reset_client(&self, client_id: NodeId) -> Result<()>;
}

pub struct PreProcessorOutputSt<O>(pub PreProcessorOutputMessage<O>, pub Instant);

pub type PreProcessorOutput<O> = PreProcessorOutputSt<O>;

pub struct BatchOutput<O>(ChannelSyncRx<PreProcessorOutput<O>>);

/// The request output message
pub struct PreProcessorOutputMessage<O> {
    deduped_requests: Vec<StoredMessage<O>>,
}

impl<O> From<Vec<StoredMessage<O>>> for PreProcessorOutputMessage<O> {
    fn from(value: Vec<StoredMessage<O>>) -> Self {
        Self {
            deduped_requests: value,
        }
    }
}

impl<O> Deref for PreProcessorOutputMessage<O> {
    type Target = Vec<StoredMessage<O>>;

    fn deref(&self) -> &Self::Target {
        &self.deduped_requests
    }
}

impl<O> DerefMut for PreProcessorOutputMessage<O> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.deduped_requests
    }
}

impl<O> IntoIterator for PreProcessorOutputMessage<O> {
    type Item = StoredMessage<O>;
    type IntoIter = IntoIter<StoredMessage<O>>;

    fn into_iter(self) -> Self::IntoIter {
        self.deduped_requests.into_iter()
    }
}

impl<O> From<PreProcessorOutputMessage<O>> for Vec<StoredMessage<O>> {
    fn from(value: PreProcessorOutputMessage<O>) -> Self {
        value.deduped_requests
    }
}

impl<O> From<ChannelSyncRx<PreProcessorOutput<O>>> for BatchOutput<O> {
    fn from(value: ChannelSyncRx<PreProcessorOutput<O>>) -> Self {
        Self(value)
    }
}

impl<O> Deref for BatchOutput<O> {
    type Target = ChannelSyncRx<PreProcessorOutput<O>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<O> BatchOutput<O> {
    pub fn recv(&self) -> std::result::Result<PreProcessorOutputMessage<O>, RecvError> {
        let (message, instant) = self.0.recv().map_err(|_| RecvError::ChannelDc)?.into();

        metric_duration(RQ_PP_WORKER_PROPOSER_PASSING_TIME_ID, instant.elapsed());

        Ok(message)
    }

    pub fn try_recv(&self) -> std::result::Result<PreProcessorOutputMessage<O>, TryRecvError> {
        let (message, instant) = self.0.try_recv()?.into();

        metric_duration(RQ_PP_WORKER_PROPOSER_PASSING_TIME_ID, instant.elapsed());

        Ok(message)
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> std::result::Result<PreProcessorOutputMessage<O>, TryRecvError> {
        let (message, instant) = self.0.recv_timeout(timeout)?.into();

        metric_duration(RQ_PP_WORKER_PROPOSER_PASSING_TIME_ID, instant.elapsed());

        Ok(message)
    }
}

#[inline]
pub fn operation_key<O>(header: &Header, message: &O) -> u64
where
    O: SessionBased,
{
    operation_key_raw(header.from(), message.session_number())
}

#[inline]
pub fn request_sender_from_key(key: u64) -> NodeId {
    NodeId::from(key as u32)
}

#[inline]
pub fn request_session_from_key(key: u64) -> SeqNo {
    SeqNo::from((key >> 32) as u32)
}

#[inline]
pub fn operation_key_raw(from: NodeId, session: SeqNo) -> u64 {
    // both of these values are 32-bit in width
    let client_id: u64 = from.into();
    let session_id: u64 = session.into();

    // therefore this is safe, and will not delete any bits
    (session_id << 32) | client_id
}

impl<O> Clone for BatchOutput<O> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<O> From<PreProcessorOutputSt<O>> for (PreProcessorOutputMessage<O>, Instant) {
    fn from(value: PreProcessorOutputSt<O>) -> Self {
        (value.0, value.1)
    }
}

impl<O: Clone> Clone for PreProcessorOutputMessage<O> {
    fn clone(&self) -> Self {
        Self {
            deduped_requests: self.deduped_requests.clone(),
        }
    }
}

impl<O> Clone for PreProcessorOutputSt<O>
where
    O: Clone,
{
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1)
    }
}
