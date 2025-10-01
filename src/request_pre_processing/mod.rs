use anyhow::Context;
use itertools::Itertools;
use std::marker::PhantomData;
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{instrument, Level};

use atlas_common::channel;
use atlas_common::channel::mixed::{ChannelMixedRx, ChannelMixedTx};
use atlas_common::channel::sync::{new_bounded_sync, ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::Result;
use atlas_common::node_id::NodeId;
use atlas_communication::message::StoredMessage;
use atlas_core::messages::{ClientRqInfo, ForwardedRequestsMessage, SessionBased};
use atlas_core::request_pre_processing::network::RequestPreProcessingHandle;
use atlas_core::request_pre_processing::{
    BatchOutput, PreProcessorOutputMessage, RequestClientPreProcessing, RequestPProcessorAsync,
    RequestPProcessorSync, RequestPreProcessing, RequestPreProcessorTimeout, WorkPartitioner,
};
use atlas_core::timeouts::timeout::ModTimeout;
use atlas_core::timeouts::TimeoutID;
use atlas_metrics::metrics::{metric_duration, metric_increment, metric_store_count};
use atlas_smr_application::serialize::ApplicationData;

use crate::exec::RequestType;
use crate::message::OrderableMessage;
use crate::metric::{
    RQ_PP_CLIENT_COUNT_ID, RQ_PP_CLIENT_MSG_ID, RQ_PP_CLONE_RQS_ID, RQ_PP_COLLECT_PENDING_ID,
    RQ_PP_DECIDED_RQS_ID, RQ_PP_FWD_RQS_ID, RQ_PP_ORCHESTRATOR_MESSAGES_PROCESSED_ID,
    RQ_PP_TIMEOUT_RQS_ID, RQ_PP_WORKER_BATCH_SIZE_ID, RQ_PP_WORKER_STOPPED_TIME_ID,
};
use crate::request_pre_processing::worker::{
    PreProcessorWorkMessage, RequestPreProcessingWorkerHandle,
};
use crate::serialize::SMRSysMessage;
use crate::SMRReq;

mod tests;
pub mod worker;

const ORCHESTRATOR_RCV_TIMEOUT: Option<Duration> = Some(Duration::from_micros(50));
const PROPOSER_QUEUE_SIZE: usize = 1024;

const RQ_PRE_PROCESSING_ORCHESTRATOR: &str = "RQ-PRE-PROCESSING-ORCHESTRATOR";

/// Message to the request pre-processor
enum PreProcessorMessage<O> {
    /// We have received forwarded requests from other replicas.
    ForwardedRequests(StoredMessage<ForwardedRequestsMessage<O>>),
    /// We have received requests that are already decided by the system
    StoppedRequests(Vec<StoredMessage<O>>),
    /// Analyse timeout requests.
    /// Returns only timeouts that have not yet been executed
    TimeoutsReceived(
        Vec<ModTimeout>,
        ChannelSyncTx<(Vec<ModTimeout>, Vec<ModTimeout>)>,
    ),
    /// A batch of requests that has been decided by the system
    DecidedBatch(Vec<ClientRqInfo>),
    /// Collect all pending messages from all workers.
    CollectAllPendingMessages(ChannelMixedTx<Vec<StoredMessage<O>>>),
    /// Clone a vec of requests to be used
    CloneRequests(Vec<ClientRqInfo>, ChannelMixedTx<Vec<StoredMessage<O>>>),
    /// Reset the stored session and operation sequence numbers for a given client
    ResetClient(NodeId),
}

/// Request pre processor handle
pub struct RequestPreProcessor<O>(ChannelSyncTx<PreProcessorMessage<O>>);

impl<O> RequestPreProcessing<O> for RequestPreProcessor<O> {
    fn process_forwarded_requests(
        &self,
        message: StoredMessage<ForwardedRequestsMessage<O>>,
    ) -> Result<()> {
        self.0
            .send(PreProcessorMessage::ForwardedRequests(message))?;

        Ok(())
    }

    fn process_stopped_requests(&self, messages: Vec<StoredMessage<O>>) -> Result<()> {
        self.0
            .send(PreProcessorMessage::StoppedRequests(messages))
            .context("Failed to send stopped requests to pre processor")
    }

    fn process_decided_batch(&self, client_rqs: Vec<ClientRqInfo>) -> Result<()> {
        self.0
            .send(PreProcessorMessage::DecidedBatch(client_rqs))
            .context("Failed to send decided batch to pre processor")
    }
}

impl<O> RequestPreProcessorTimeout for RequestPreProcessor<O> {
    fn process_timeouts(
        &self,
        timeouts: Vec<ModTimeout>,
        response_channel: ChannelSyncTx<(Vec<ModTimeout>, Vec<ModTimeout>)>,
    ) -> Result<()> {
        self.0
            .send(PreProcessorMessage::TimeoutsReceived(
                timeouts,
                response_channel,
            ))
            .context("Failed to send timeouts to pre processor")
    }
}

impl<O> RequestPProcessorAsync<O> for RequestPreProcessor<O> {
    fn clone_pending_rqs(
        &self,
        client_rqs: Vec<ClientRqInfo>,
    ) -> Result<ChannelMixedRx<Vec<StoredMessage<O>>>> {
        let (tx, rx) =
            channel::mixed::new_bounded_mixed(self.0.len(), Some("Clone Pending Requests"));

        self.0
            .send(PreProcessorMessage::CloneRequests(client_rqs, tx))?;

        Ok(rx)
    }

    fn collect_pending_rqs(&self) -> Result<ChannelMixedRx<Vec<StoredMessage<O>>>> {
        let (tx, rx) =
            channel::mixed::new_bounded_mixed(self.0.len(), Some("Clone Pending Requests"));

        self.0
            .send(PreProcessorMessage::CollectAllPendingMessages(tx))?;

        Ok(rx)
    }
}

impl<O> RequestPProcessorSync<O> for RequestPreProcessor<O> {
    fn clone_pending_rqs(&self, client_rqs: Vec<ClientRqInfo>) -> Result<Vec<StoredMessage<O>>> {
        let channel = <Self as RequestPProcessorAsync<O>>::clone_pending_rqs(self, client_rqs)?;

        let mut cloned_rqs = Vec::new();

        while let Ok(mut message) = channel.recv() {
            cloned_rqs.append(&mut message);
        }

        Ok(cloned_rqs)
    }

    fn collect_pending_rqs(&self) -> Result<Vec<StoredMessage<O>>> {
        let channel = <Self as RequestPProcessorAsync<O>>::collect_pending_rqs(self)?;

        let mut collected_rqs = Vec::new();

        while let Ok(mut message) = channel.recv() {
            collected_rqs.append(&mut message);
        }

        Ok(collected_rqs)
    }
}

impl<O> RequestClientPreProcessing for RequestPreProcessor<O> {
    fn reset_client(&self, client_id: NodeId) -> Result<()> {
        self.0
            .send(PreProcessorMessage::ResetClient(client_id))
            .context("Failed to send reset client to pre processor")
    }
}

impl<O> From<ChannelSyncTx<PreProcessorMessage<O>>> for RequestPreProcessor<O> {
    fn from(value: ChannelSyncTx<PreProcessorMessage<O>>) -> Self {
        Self(value)
    }
}

/// The orchestrator for all of the request pre processing.
/// Decides which workers will get which requests and then handles the logic necessary
struct RequestPreProcessingOrchestrator<WD, D, NT>
where
    D: ApplicationData + 'static,
{
    /// How many workers should we have
    thread_count: usize,
    /// Work message transmission for each worker
    work_comms: Vec<RequestPreProcessingWorkerHandle<D>>,
    /// The RX end for a work channel for the request pre processor
    ordered_work_receiver: ChannelSyncRx<PreProcessorMessage<SMRReq<D>>>,
    /// The network node so we can poll messages received from the clients
    network_node: Arc<NT>,
    /// How we are going to divide the work between workers
    work_divider: PhantomData<fn() -> WD>,
}

impl<WD, D, NT> RequestPreProcessingOrchestrator<WD, D, NT>
where
    D: ApplicationData,
    WD: Send,
{
    fn run(mut self)
    where
        D: ApplicationData,
        NT: RequestPreProcessingHandle<SMRSysMessage<D>>,
        WD: WorkPartitioner,
    {
        loop {
            self.process_client_rqs();
            self.process_work_messages();
        }
    }

    #[instrument(skip(self), level = Level::DEBUG)]
    fn process_client_rqs(&mut self)
    where
        D: ApplicationData,
        NT: RequestPreProcessingHandle<SMRSysMessage<D>>,
        WD: WorkPartitioner,
    {
        let messages = match self
            .network_node
            .receive_from_clients(ORCHESTRATOR_RCV_TIMEOUT)
        {
            Ok(message) => message,
            Err(_) => {
                return;
            }
        };

        metric_store_count(RQ_PP_WORKER_BATCH_SIZE_ID, messages.len());

        let start = Instant::now();
        let msg_count = messages.len();

        if !messages.is_empty() {
            messages
                .into_iter()
                .chunk_by(|message| {
                    WD::get_worker_for_raw(
                        message.header().from(),
                        message.message().session_number(),
                        self.thread_count,
                    )
                })
                .into_iter()
                .for_each(|(worker, messages)| {
                    self.work_comms[worker].send(
                        PreProcessorWorkMessage::ClientPoolRequestsReceived(messages.collect()),
                    );
                });

            metric_duration(RQ_PP_CLIENT_MSG_ID, start.elapsed());
        }

        metric_increment(RQ_PP_CLIENT_COUNT_ID, Some(msg_count as u64));
    }

    #[instrument(skip(self), level = Level::DEBUG)]
    fn process_work_messages(&mut self)
    where
        D: ApplicationData,
        WD: WorkPartitioner,
    {
        while let Ok(work_recved) = self.ordered_work_receiver.try_recv() {
            match work_recved {
                PreProcessorMessage::ForwardedRequests(fwd_reqs) => {
                    self.process_forwarded_rqs(RequestType::Ordered, fwd_reqs);
                }
                PreProcessorMessage::DecidedBatch(decided) => {
                    self.process_decided_batch(decided);
                }
                PreProcessorMessage::TimeoutsReceived(timeouts, responder) => {
                    self.process_timeouts(timeouts, responder);
                }
                PreProcessorMessage::CollectAllPendingMessages(tx) => {
                    self.collect_pending_rqs(RequestType::Ordered, tx);
                }
                PreProcessorMessage::StoppedRequests(stopped) => {
                    self.process_stopped_rqs(RequestType::Ordered, stopped);
                }
                PreProcessorMessage::CloneRequests(client_rqs, tx) => {
                    self.clone_pending_rqs(client_rqs, tx);
                }
                PreProcessorMessage::ResetClient(client_id) => {
                    self.process_reset_client(client_id);
                }
            }

            metric_increment(RQ_PP_ORCHESTRATOR_MESSAGES_PROCESSED_ID, None);
        }
    }

    fn process_reset_client(&self, node: NodeId) {
        self.work_comms
            .iter()
            .for_each(|worker| worker.send(PreProcessorWorkMessage::CleanClient(node)));
    }

    fn process_forwarded_rqs(
        &self,
        request_type: RequestType,
        fwd_reqs: StoredMessage<ForwardedRequestsMessage<SMRReq<D>>>,
    ) where
        D: ApplicationData,
        WD: WorkPartitioner,
    {
        let start = Instant::now();

        let (_, message) = fwd_reqs.into_inner();

        let fwd_reqs = message.into_inner();

        fwd_reqs
            .into_iter()
            .map(|msg| map_smr_req(msg, request_type))
            .chunk_by(|message| {
                WD::get_worker_for_raw(
                    message.header().from(),
                    message.message().session_number(),
                    self.thread_count,
                )
            })
            .into_iter()
            .for_each(|(worker, messages)| {
                self.work_comms[worker].send(PreProcessorWorkMessage::ForwardedRequestsReceived(
                    messages.collect(),
                ))
            });

        metric_duration(RQ_PP_FWD_RQS_ID, start.elapsed());
    }

    fn process_decided_batch(&self, client_rqs: Vec<ClientRqInfo>)
    where
        D: ApplicationData,
        WD: WorkPartitioner,
    {
        let start = Instant::now();

        client_rqs
            .into_iter()
            .chunk_by(|rq| WD::get_worker_for_processed(rq, self.thread_count))
            .into_iter()
            .for_each(|(worker, rqs)| {
                self.work_comms[worker].send(PreProcessorWorkMessage::DecidedBatch(rqs.collect()))
            });

        metric_duration(RQ_PP_DECIDED_RQS_ID, start.elapsed());
    }

    fn process_timeouts(
        &self,
        timeouts: Vec<ModTimeout>,
        responder: ChannelSyncTx<(Vec<ModTimeout>, Vec<ModTimeout>)>,
    ) where
        D: ApplicationData,
        WD: WorkPartitioner,
    {
        let start = Instant::now();

        timeouts
            .into_iter()
            .chunk_by(|timeout| {
                if let TimeoutID::SessionBased { from, session, .. } = timeout.id() {
                    WD::get_worker_for_raw(*from, *session, self.thread_count)
                } else {
                    0
                }
            })
            .into_iter()
            .for_each(|(worker, work)| {
                self.work_comms[worker].send(PreProcessorWorkMessage::TimeoutsReceived(
                    work.collect(),
                    responder.clone(),
                ))
            });

        metric_duration(RQ_PP_TIMEOUT_RQS_ID, start.elapsed());
    }

    /// Process stopped requests by forwarding them to the appropriate worker.
    fn process_stopped_rqs(&self, rq_type: RequestType, rqs: Vec<StoredMessage<SMRReq<D>>>)
    where
        D: ApplicationData,
        WD: WorkPartitioner,
    {
        let start = Instant::now();

        rqs.into_iter()
            .map(|msg| map_smr_req(msg, rq_type))
            .chunk_by(|message| {
                WD::get_worker_for_raw(
                    message.header().from(),
                    message.message().session_number(),
                    self.thread_count,
                )
            })
            .into_iter()
            .for_each(|(worker, messages)| {
                self.work_comms[worker].send(PreProcessorWorkMessage::StoppedRequestsReceived(
                    messages.collect(),
                ))
            });

        metric_duration(RQ_PP_WORKER_STOPPED_TIME_ID, start.elapsed());
    }

    fn collect_pending_rqs(
        &self,
        request_type: RequestType,
        tx: ChannelMixedTx<Vec<StoredMessage<SMRReq<D>>>>,
    ) {
        let start = Instant::now();

        self.work_comms.iter().for_each(|worker| {
            worker.send(PreProcessorWorkMessage::CollectPendingMessages(
                request_type,
                tx.clone(),
            ))
        });

        metric_duration(RQ_PP_COLLECT_PENDING_ID, start.elapsed());
    }

    fn clone_pending_rqs(
        &self,
        digests: Vec<ClientRqInfo>,
        responder: ChannelMixedTx<Vec<StoredMessage<SMRReq<D>>>>,
    ) where
        D: ApplicationData,
        WD: WorkPartitioner,
    {
        let start = Instant::now();

        digests
            .into_iter()
            .chunk_by(|rq| WD::get_worker_for_processed(rq, self.thread_count))
            .into_iter()
            .for_each(|(worker, rqs)| {
                self.work_comms[worker].send(PreProcessorWorkMessage::ClonePendingRequests(
                    rqs.collect(),
                    RequestType::Ordered,
                    responder.clone(),
                ))
            });

        metric_duration(RQ_PP_CLONE_RQS_ID, start.elapsed());
    }
}

pub fn initialize_request_pre_processor<WD, D, NT>(
    concurrency: usize,
    node: &Arc<NT>,
) -> (OrderedRqHandles<SMRReq<D>>, UnorderedRqHandles<SMRReq<D>>)
where
    D: ApplicationData + Send + 'static,
    NT: RequestPreProcessingHandle<SMRSysMessage<D>> + 'static,
    WD: WorkPartitioner + 'static,
{
    let (batch_tx, receiver) =
        new_bounded_sync(PROPOSER_QUEUE_SIZE, Some("Pre Processor Batch Output"));

    let (unordered_batch_tx, unordered_receiver) = new_bounded_sync(
        PROPOSER_QUEUE_SIZE,
        Some("Pre Processor Unordered Batch Output"),
    );

    let (work_sender, work_rcvr) =
        new_bounded_sync(PROPOSER_QUEUE_SIZE, Some("Pre Processor Work handle"));

    let mut work_comms = Vec::with_capacity(concurrency);

    for worker_id in 0..concurrency {
        let worker_handle =
            worker::spawn_worker(worker_id, batch_tx.clone(), unordered_batch_tx.clone());

        work_comms.push(worker_handle);
    }

    let orchestrator = RequestPreProcessingOrchestrator::<WD, D, NT> {
        thread_count: concurrency,
        work_comms,
        ordered_work_receiver: work_rcvr,
        network_node: node.clone(),
        work_divider: Default::default(),
    };

    launch_orchestrator_thread(orchestrator);

    (
        OrderedRqHandles(work_sender.into(), receiver.into()),
        UnorderedRqHandles(unordered_receiver.into()),
    )
}

fn launch_orchestrator_thread<WD, D, NT>(orchestrator: RequestPreProcessingOrchestrator<WD, D, NT>)
where
    D: ApplicationData + Send + 'static,
    NT: RequestPreProcessingHandle<SMRSysMessage<D>> + 'static,
    WD: WorkPartitioner + 'static,
{
    std::thread::Builder::new()
        .name(RQ_PRE_PROCESSING_ORCHESTRATOR.to_string())
        .spawn(move || {
            orchestrator.run();
        })
        .expect("Failed to launch orchestrator thread.");
}

pub struct OrderedRqHandles<O>(RequestPreProcessor<O>, BatchOutput<O>);

pub struct UnorderedRqHandles<O>(BatchOutput<O>);

impl<O> From<OrderedRqHandles<O>> for (RequestPreProcessor<O>, BatchOutput<O>) {
    #[instrument(skip_all)]
    fn from(value: OrderedRqHandles<O>) -> Self {
        (value.0, value.1)
    }
}

impl<O> From<UnorderedRqHandles<O>> for BatchOutput<O> {
    #[instrument(skip_all)]
    fn from(value: UnorderedRqHandles<O>) -> Self {
        value.0
    }
}

impl<O> Deref for UnorderedRqHandles<O> {
    type Target = BatchOutput<O>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn map_smr_req<D>(
    request: StoredMessage<SMRReq<D>>,
    request_type: RequestType,
) -> StoredMessage<SMRSysMessage<D>>
where
    D: ApplicationData + 'static,
{
    let (header, msg) = request.into_inner();

    let msg = match request_type {
        RequestType::Ordered => OrderableMessage::OrderedRequest(msg),
        RequestType::Unordered => OrderableMessage::UnorderedRequest(msg),
    };

    StoredMessage::new(header, msg)
}

impl<O> Clone for RequestPreProcessor<O> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<O> Clone for OrderedRqHandles<O> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1.clone())
    }
}
