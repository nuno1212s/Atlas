use intmap::IntMap;
use std::time::Instant;

use tracing::{debug, error, info, trace};

use crate::exec::RequestType;
use crate::message::OrderableMessage;
use atlas_common::channel::mixed::ChannelMixedTx;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::channel::TrySendReturnError;
use atlas_common::collections::HashMap;
use atlas_common::crypto::hash::Digest;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::{Header, StoredMessage};
use atlas_core::messages::SessionBased;
use atlas_core::messages::{create_rq_correlation_id, ClientRqInfo};
use atlas_core::metric::{RQ_CLIENT_TRACKING_ID, RQ_CLIENT_TRACK_GLOBAL_ID};
use atlas_core::request_pre_processing::{
    operation_key, operation_key_raw, request_sender_from_key, PreProcessorOutput,
    PreProcessorOutputSt,
};
use atlas_core::timeouts::timeout::ModTimeout;
use atlas_core::timeouts::TimeoutID;
use atlas_metrics::metrics::{
    metric_correlation_time_start, metric_duration, metric_increment,
    metric_initialize_correlation_id,
};
use atlas_smr_application::serialize::ApplicationData;

use crate::metric::{
    CLIENT_RQ_ENTER_RQ_PRE_PROCESSOR, RQ_PP_ORCHESTRATOR_WORKER_PASSING_TIME_ID,
    RQ_PP_WORKER_DECIDED_PROCESS_TIME_ID, RQ_PP_WORKER_DISCARDED_RQS_ID,
    RQ_PP_WORKER_ORDER_PROCESS_COUNT_ID, RQ_PP_WORKER_ORDER_PROCESS_ID,
};
use crate::request_pre_processing::PreProcessorOutputMessage;
use crate::serialize::SMRSysMessage;
use crate::SMRReq;

const WORKER_QUEUE_SIZE: usize = 1024;
const WORKER_THREAD_NAME: &str = "RQ-PRE-PROCESSING-WORKER-{}";

#[derive(Clone)]
pub struct PreProcessorWorkMessageSt<O: ApplicationData + 'static>(
    Instant,
    PreProcessorWorkMessage<O>,
);

impl<O> PreProcessorWorkMessageSt<O>
where
    O: ApplicationData + 'static,
{
    pub fn new(message: PreProcessorWorkMessage<O>) -> Self {
        Self(Instant::now(), message)
    }
}

pub type PreProcessorWorkMessageOuter<O: ApplicationData + 'static> = PreProcessorWorkMessageSt<O>;

#[derive(Clone)]
pub enum PreProcessorWorkMessage<D>
where
    D: ApplicationData + 'static,
{
    /// We have received requests from the clients, which need
    /// to be processed
    ClientPoolRequestsReceived(Vec<StoredMessage<SMRSysMessage<D>>>),
    /// Received requests that were forwarded from other replicas
    ForwardedRequestsReceived(Vec<StoredMessage<SMRSysMessage<D>>>),
    StoppedRequestsReceived(Vec<StoredMessage<SMRSysMessage<D>>>),
    /// Analyse timeout requests. Returns only timeouts that have not yet been executed
    TimeoutsReceived(
        Vec<ModTimeout>,
        ChannelSyncTx<(Vec<ModTimeout>, Vec<ModTimeout>)>,
    ),
    /// A batch of requests has been decided by the system
    DecidedBatch(Vec<ClientRqInfo>),
    /// Collect all pending messages from the given worker
    CollectPendingMessages(RequestType, ChannelMixedTx<Vec<StoredMessage<SMRReq<D>>>>),
    /// Clone a set of given pending requests
    ClonePendingRequests(
        Vec<ClientRqInfo>,
        RequestType,
        ChannelMixedTx<Vec<StoredMessage<SMRReq<D>>>>,
    ),
    /// Remove all requests associated with this client (due to a disconnection, for example)
    CleanClient(NodeId),
}

struct BatchProduction<O> {
    pending_request_limit: usize,
    pending_requests: Option<Vec<StoredMessage<O>>>,
    production_tx: ChannelSyncTx<PreProcessorOutput<O>>,
}

/// Each worker will be assigned a given set of clients
pub struct RequestPreProcessingWorker<D>
where
    D: ApplicationData + 'static,
{
    worker_id: usize,
    /// Receive work
    message_rx: ChannelSyncRx<PreProcessorWorkMessageOuter<D>>,

    /// Output for the requests that have been processed and should now be proposed
    ordered_batch_prod: BatchProduction<SMRReq<D>>,

    unordered_batch_prod: BatchProduction<SMRReq<D>>,

    /// The latest operations seen by this worker.
    /// Since a given session will always be handled by the same worker,
    /// we can use this to filter out duplicates.
    latest_ops: IntMap<(SeqNo, Option<Digest>)>,
    /// The requests that have not been added to a batch yet.
    pending_requests: HashMap<Digest, StoredMessage<SMRSysMessage<D>>>,
}

impl<D> RequestPreProcessingWorker<D>
where
    D: ApplicationData + 'static,
{
    pub fn new(
        worker_id: usize,
        message_rx: ChannelSyncRx<PreProcessorWorkMessageOuter<D>>,
        ordered_batch_production: ChannelSyncTx<PreProcessorOutput<SMRReq<D>>>,
        unordered_batch_production: ChannelSyncTx<PreProcessorOutput<SMRReq<D>>>,
    ) -> Self {
        Self {
            worker_id,
            message_rx,
            ordered_batch_prod: ordered_batch_production.into(),
            unordered_batch_prod: unordered_batch_production.into(),
            latest_ops: Default::default(),
            pending_requests: Default::default(),
        }
    }

    pub fn run(mut self) {
        loop {
            let (sent_time, recvd_message) = self.message_rx.recv().unwrap().into();

            match recvd_message {
                PreProcessorWorkMessage::ClientPoolRequestsReceived(requests) => {
                    self.process_client_pool_requests(requests);
                }
                PreProcessorWorkMessage::ForwardedRequestsReceived(requests) => {
                    self.process_forwarded_requests(requests);
                }
                PreProcessorWorkMessage::TimeoutsReceived(requests, tx) => {
                    self.process_timeouts(requests, tx);
                }
                PreProcessorWorkMessage::DecidedBatch(requests) => {
                    self.process_decided_batch(requests);
                }
                PreProcessorWorkMessage::CollectPendingMessages(rq_type, tx) => {
                    let reqs = self.collect_pending_requests(rq_type);

                    tx.send(reqs).expect("Failed to send pending requests");
                }
                PreProcessorWorkMessage::ClonePendingRequests(requests, rq_type, tx) => {
                    self.clone_pending_requests(rq_type, requests, tx);
                }
                PreProcessorWorkMessage::CleanClient(client) => {
                    self.clean_client(client);
                }
                PreProcessorWorkMessage::StoppedRequestsReceived(reqs) => {
                    self.stopped_requests(reqs);
                }
            }

            metric_duration(
                RQ_PP_ORCHESTRATOR_WORKER_PASSING_TIME_ID,
                sent_time.elapsed(),
            );
        }
    }

    fn check_latest_op_for_key(&mut self, key: u64) -> (SeqNo, Option<Digest>) {
        if let Some((seq_no, digest)) = self.latest_ops.get_mut(key) {
            (*seq_no, *digest)
        } else {
            (SeqNo::ZERO, None)
        }
    }

    /// Checks if we have received a more recent message for a given client/session combo
    fn has_received_more_recent_and_update(
        &mut self,
        header: &Header,
        message: &SMRSysMessage<D>,
        unique_digest: &Digest,
    ) -> bool {
        let key = operation_key::<SMRSysMessage<D>>(header, message);

        let (seq_no, digest) = self.check_latest_op_for_key(key);

        let has_received_more_recent = seq_no >= message.sequence_number();

        if !has_received_more_recent {
            digest.map(|digest| self.pending_requests.remove(&digest));

            self.latest_ops
                .insert(key, (message.sequence_number(), Some(*unique_digest)));
        }

        has_received_more_recent
    }

    fn update_most_recent(&mut self, rq_info: &ClientRqInfo) {
        let key = operation_key_raw(rq_info.sender, rq_info.session);

        let (seq_no, digest) = self.check_latest_op_for_key(key);

        let has_received_more_recent = seq_no >= rq_info.seq_no;

        if !has_received_more_recent {
            digest.map(|digest| self.pending_requests.remove(&digest));

            self.latest_ops.insert(key, (rq_info.seq_no, None));
        }
    }

    /// Process the ordered client pool requests
    //#[instrument(skip_all, level = "debug", fields(worker_id = self.worker_id, request_len = requests.len()))]
    pub fn process_client_pool_requests(&mut self, requests: Vec<StoredMessage<SMRSysMessage<D>>>) {
        let start = Instant::now();

        trace!(
            "Worker {} // Processing client pool requests {}",
            self.worker_id,
            requests.len()
        );

        let processed_rqs = requests.len();

        let mut ordered_requests = Vec::with_capacity(requests.len());

        let mut unordered_requests = Vec::with_capacity(requests.len());

        let mut discarded_requests = 0;

        for message in requests.into_iter() {
            let digest = message.header().unique_digest();

            if self.has_received_more_recent_and_update(
                message.header(),
                message.message(),
                &digest,
            ) {
                debug!("Discarding request {:?} as it is not the most recent for session {:?} of client {:?}.",
                message.message().sequence_number(), message.message().session_number(), message.header().from());

                discarded_requests += 1;
                continue;
            }

            metric_initialize_correlation_id(
                RQ_CLIENT_TRACKING_ID,
                create_rq_correlation_id(message.header().from(), message.message()),
                CLIENT_RQ_ENTER_RQ_PRE_PROCESSOR.clone(),
            );

            metric_correlation_time_start(
                RQ_CLIENT_TRACK_GLOBAL_ID,
                create_rq_correlation_id(message.header().from(), message.message()),
            );

            match &message.message() {
                OrderableMessage::OrderedRequest(_) => {
                    let header = *message.header();

                    let req_msg = message.message().clone().into_smr_request();

                    ordered_requests.push(StoredMessage::new(header, req_msg));

                    self.pending_requests.insert(digest, message);
                }
                OrderableMessage::UnorderedRequest(_) => {
                    let (header, message) = message.into_inner();

                    unordered_requests.push(StoredMessage::new(header, message.into_smr_request()));
                }
                _ => {
                    error!("Received a reply as a client pool message.");
                    continue;
                }
            }
        }

        if !ordered_requests.is_empty() {
            self.ordered_batch_prod.send(ordered_requests);
        }

        if !unordered_requests.is_empty() {
            self.unordered_batch_prod.send(unordered_requests);
        }

        metric_duration(RQ_PP_WORKER_ORDER_PROCESS_ID, start.elapsed());
        metric_increment(
            RQ_PP_WORKER_ORDER_PROCESS_COUNT_ID,
            Some(processed_rqs as u64),
        );
        metric_increment(
            RQ_PP_WORKER_DISCARDED_RQS_ID,
            Some(discarded_requests as u64),
        );
    }

    /// Process the forwarded requests
    //#[instrument(skip_all, level = "debug", fields(worker_id = self.worker_id, request_len = requests.len()))]
    pub fn process_forwarded_requests(&mut self, requests: Vec<StoredMessage<SMRSysMessage<D>>>) {
        let initial_size = requests.len();

        let requests: Vec<StoredMessage<SMRReq<D>>> = requests
            .into_iter()
            .filter(|request| {
                let digest = request.header().unique_digest();

                if self.has_received_more_recent_and_update(
                    request.header(),
                    request.message(),
                    &digest,
                ) {
                    return false;
                }

                match request.message() {
                    OrderableMessage::OrderedRequest(_) => {
                        self.pending_requests.insert(digest, request.clone());
                    }
                    OrderableMessage::UnorderedRequest(_) => {
                        error!("Received an unordered request as a forwarded message.");
                        return false;
                    }
                    _ => {
                        error!("Received a reply as a forwarded message.");
                        return false;
                    }
                }

                true
            })
            .map(|req| {
                let (header, message) = req.into_inner();

                StoredMessage::new(header, message.into_smr_request())
            })
            .collect();

        debug!(
            "Worker {} // Forwarded requests processed, out of {} left with {:?}",
            self.worker_id,
            initial_size,
            requests.len()
        );

        if !requests.is_empty() {
            self.ordered_batch_prod.send(requests);
        }
    }

    /// Process the timeouts
    /// We want that timeouts which are either for requests we have not yet seen
    /// And for requests that we have seen and are still in the pending request list.
    /// If they are not in the pending request map that means they have already been executed
    /// And need not be processed
    //#[instrument(skip_all, level = "debug", fields(worker_id = self.worker_id, timeouts = timeouts.len()))]
    fn process_timeouts(
        &mut self,
        timeouts: Vec<ModTimeout>,
        tx: ChannelSyncTx<(Vec<ModTimeout>, Vec<ModTimeout>)>,
    ) {
        let mut returned_timeouts = Vec::with_capacity(timeouts.len());

        let mut removed_timeouts = Vec::with_capacity(timeouts.len());

        for timeout in timeouts {
            let result = if let TimeoutID::SessionBased {
                from,
                session,
                seq_no,
            } = timeout.id()
            {
                if timeout.extra_info().is_none() {
                    debug!("Ignoring timeout {:?} as it has no extra info.", timeout);

                    continue;
                }

                let rq_info: Option<&ClientRqInfo> =
                    <dyn std::any::Any>::downcast_ref::<ClientRqInfo>(
                        timeout.extra_info().unwrap().as_any(),
                    );

                if rq_info.is_none() {
                    continue;
                }

                let rq_info = rq_info.unwrap();

                let key = operation_key_raw(*from, *session);

                if let Some((cur_seq_no, _)) = self.latest_ops.get(key) {
                    if *cur_seq_no >= *seq_no {
                        self.pending_requests.contains_key(&rq_info.digest)
                    } else {
                        true
                    }
                } else {
                    true
                }
            } else {
                false
            };

            if result {
                returned_timeouts.push(timeout);
            } else {
                removed_timeouts.push(timeout);
            }
        }

        tx.send((returned_timeouts, removed_timeouts))
            .expect("Failed to send timeouts to client");
    }

    /// Process a decided batch
    //#[instrument(skip_all, level = "debug", fields(worker_id = self.worker_id, request_len = requests.len()))]
    fn process_decided_batch(&mut self, requests: Vec<ClientRqInfo>) {
        let start = Instant::now();

        requests.into_iter().for_each(|request| {
            self.pending_requests.remove(&request.digest);

            // Update so that if we later on receive the same request from the client, we can safely ignore it
            // And not get build up in the pending requests
            self.update_most_recent(&request);
        });

        metric_duration(RQ_PP_WORKER_DECIDED_PROCESS_TIME_ID, start.elapsed());
    }

    //#[instrument(skip_all, level = "debug", fields(worker_id = self.worker_id, request_len = requests.len()))]
    /// Clone a set of pending requests
    fn clone_pending_requests(
        &self,
        rq_type: RequestType,
        requests: Vec<ClientRqInfo>,
        responder: ChannelMixedTx<Vec<StoredMessage<SMRReq<D>>>>,
    ) {
        let final_rqs = requests
            .into_iter()
            .filter_map(|rq_info| self.pending_requests.get(&rq_info.digest))
            .map(Clone::clone)
            .filter(|rq| filter_message_type(rq.message(), rq_type))
            .map(|rq| {
                let (header, msg) = rq.into_inner();

                StoredMessage::new(header, msg.into_smr_request())
            })
            .collect();

        responder
            .send(final_rqs)
            .expect("Failed to send pending requests");
    }

    /// Collect all pending requests stored in this worker
    fn collect_pending_requests(&mut self, rq_type: RequestType) -> Vec<StoredMessage<SMRReq<D>>> {
        std::mem::take(&mut self.pending_requests)
            .into_values()
            .filter(|rq| filter_message_type(rq.message(), rq_type))
            .map(|rq| {
                let (header, msg) = rq.into_inner();

                StoredMessage::new(header, msg.into_smr_request())
            })
            .collect()
    }

    fn clean_client(&mut self, node_id: NodeId) {
        info!(
            "Cleaning client {:?} from worker {}",
            node_id, self.worker_id
        );

        self.pending_requests
            .retain(|_, msg| msg.header().from() != node_id);

        self.latest_ops
            .retain(|key, (_, _)| request_sender_from_key(key) != node_id);
    }

    //#[instrument(skip_all, level = "debug", fields(worker_id = self.worker_id, request_len = requests.len()))]
    fn stopped_requests(&mut self, requests: Vec<StoredMessage<SMRSysMessage<D>>>) {
        requests.into_iter().for_each(|request| {
            let digest = request.header().unique_digest();

            if self.has_received_more_recent_and_update(
                request.header(),
                request.message(),
                &digest,
            ) {
                return;
            }

            self.pending_requests.insert(digest, request.clone());
        })
    }
}

fn filter_message_type<D>(message: &OrderableMessage<D>, request_type: RequestType) -> bool
where
    D: ApplicationData,
{
    match message {
        OrderableMessage::OrderedRequest(_) if matches!(request_type, RequestType::Ordered) => true,
        OrderableMessage::UnorderedRequest(_) if matches!(request_type, RequestType::Unordered) => {
            true
        }
        OrderableMessage::OrderedRequest(_) => false,
        OrderableMessage::UnorderedRequest(_) => false,
        _ => unreachable!(),
    }
}

impl<O> BatchProduction<O> {
    fn append_to_pending(&mut self, mut rqs: Vec<StoredMessage<O>>) {
        if let Some(pending_rqs) = self.pending_requests.as_mut() {
            if rqs.len() + pending_rqs.len() > self.pending_request_limit {
                pending_rqs.append(
                    &mut rqs
                        .drain(..self.pending_request_limit - pending_rqs.len())
                        .collect(),
                );
            } else {
                pending_rqs.append(&mut rqs);
            }
        } else if rqs.len() > self.pending_request_limit {
            self.pending_requests = Some(rqs.drain(..self.pending_request_limit).collect());
        } else {
            self.pending_requests = Some(rqs);
        }
    }

    fn send(&mut self, requests: Vec<StoredMessage<O>>)
    where
        O: Clone,
    {
        let requests = match self.pending_requests.take() {
            Some(mut pending) => {
                pending.extend(requests);
                pending
            }
            None => requests,
        };

        match self.production_tx.try_send_return(PreProcessorOutputSt(
            PreProcessorOutputMessage::from(requests),
            Instant::now(),
        )) {
            Ok(_) => {}
            Err(err) => match err {
                TrySendReturnError::Full(messages) => {
                    debug!(
                        "Batch production is full, pending requests: {}",
                        messages.0.len()
                    );

                    self.append_to_pending(messages.0.into());
                }
                TrySendReturnError::Disconnected(_) | TrySendReturnError::Timeout(_) => {
                    error!("Failed to send requests to batch production: {:?}", err);
                }
            },
        }
    }
}

impl<O> From<ChannelSyncTx<PreProcessorOutput<O>>> for BatchProduction<O> {
    fn from(value: ChannelSyncTx<PreProcessorOutput<O>>) -> Self {
        Self {
            pending_request_limit: 16384,
            pending_requests: None,
            production_tx: value,
        }
    }
}

pub(super) fn spawn_worker<D>(
    worker_id: usize,
    batch_tx: ChannelSyncTx<PreProcessorOutput<SMRReq<D>>>,
    unordered_batch_rx: ChannelSyncTx<PreProcessorOutput<SMRReq<D>>>,
) -> RequestPreProcessingWorkerHandle<D>
where
    D: ApplicationData + 'static,
{
    let (worker_tx, worker_rx) = atlas_common::channel::sync::new_bounded_sync(
        WORKER_QUEUE_SIZE,
        Some(format!("RQ PreProcessing Worker Handle {worker_id}").as_str()),
    );

    let worker =
        RequestPreProcessingWorker::new(worker_id, worker_rx, batch_tx, unordered_batch_rx);

    std::thread::Builder::new()
        .name(format!("{WORKER_THREAD_NAME}{worker_id}"))
        .spawn(move || {
            worker.run();
        })
        .expect("Failed to spawn worker thread");

    RequestPreProcessingWorkerHandle(worker_tx)
}

pub struct RequestPreProcessingWorkerHandle<D>(ChannelSyncTx<PreProcessorWorkMessageOuter<D>>)
where
    D: ApplicationData + 'static;

impl<D> RequestPreProcessingWorkerHandle<D>
where
    D: ApplicationData + 'static,
{
    pub fn send(&self, message: PreProcessorWorkMessage<D>) {
        self.0
            .send(PreProcessorWorkMessageSt(Instant::now(), message))
            .unwrap()
    }
}

impl<D> From<PreProcessorWorkMessageOuter<D>> for (Instant, PreProcessorWorkMessage<D>)
where
    D: ApplicationData,
{
    fn from(value: PreProcessorWorkMessageOuter<D>) -> (Instant, PreProcessorWorkMessage<D>) {
        (value.0, value.1)
    }
}
