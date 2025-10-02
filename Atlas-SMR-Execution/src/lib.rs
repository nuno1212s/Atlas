#![allow(incomplete_features)]
#![feature(specialization)]

use crate::metric::{REPLIES_SENT_TIME_ID, REPLYING_TO_REQUEST};
use crate::scalable::{CRUDState, ScalableApp};
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::ordering::SeqNo;
use atlas_common::threadpool;
use atlas_core::messages::{create_rq_correlation_id_from_parts, ReplyMessage};
use atlas_core::metric::{RQ_CLIENT_TRACKING_ID, RQ_CLIENT_TRACK_GLOBAL_ID};
use atlas_metrics::metrics::{
    metric_correlation_id_ended, metric_correlation_time_end, metric_duration,
};
use atlas_smr_application::app::{Application, BatchReplies, Request};
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_application::state::divisible_state::DivisibleState;
use atlas_smr_application::state::monolithic_state::MonolithicState;
use atlas_smr_application::state::{divisible_state, monolithic_state};
use atlas_smr_application::{ExecutionRequest, ExecutorHandle};
use atlas_smr_core::exec::{ReplyNode, RequestType};
use atlas_smr_core::SMRReply;
use std::sync::Arc;
use std::time::Instant;
use tracing::error;

pub mod metric;
pub mod scalable;
pub mod single_threaded;

pub struct SingleThreadedMonExecutor;

pub struct MultiThreadedMonExecutor;

pub struct SingleThreadedDivExecutor;

pub struct MultiThreadedDivExecutor;

pub type ExecutorHandles<A, S> = (
    ExecutorHandle<Request<A, S>>,
    ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
);

pub type DVStateInstallHandle<S> = (
    ChannelSyncTx<divisible_state::InstallStateMessage<S>>,
    ChannelSyncRx<divisible_state::AppStateMessage<S>>,
);

/// Trait defining the necessary methods for a divisible state executor
/// (In reality since all communication is done via channels, this ends up just defining the
/// initialisation method)
pub trait TDivisibleStateExecutor<A, S, NT>
where
    A: Application<S> + 'static,
    S: DivisibleState + 'static,
    NT: 'static,
{
    /// Initialize a handle and a channel to receive requests
    fn init_handle() -> ExecutorHandles<A, S>;

    /// Initialization method for the executor
    /// Should return a channel for the state messages to be sent to the executor
    /// As well as a channel to receive checkpoints from the application
    fn init(
        work_receiver: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<DVStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static;
}

pub type MonStateInstallHandle<S> = (
    ChannelSyncTx<monolithic_state::InstallStateMessage<S>>,
    ChannelSyncRx<monolithic_state::AppStateMessage<S>>,
);
/// Trait defining the necessary methods for a monolithic state executor
/// (In reality since all communication is done via channels, this ends up just defining the
/// initialisation method)
pub trait TMonolithicStateExecutor<A, S, NT>
where
    A: Application<S> + 'static,
    S: MonolithicState + 'static,
    NT: 'static,
{
    /// Initialize a handle and a channel to receive requests
    fn init_handle() -> ExecutorHandles<A, S>;

    /// Initialization method for the executor
    /// Should return a channel for the state messages to be sent to the executor
    /// As well as a channel to receive checkpoints from the application
    fn init(
        work_receiver: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<MonStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static;
}

impl<A, S, NT> TDivisibleStateExecutor<A, S, NT> for SingleThreadedDivExecutor
where
    A: Application<S> + 'static,
    S: DivisibleState + Send + 'static,
    NT: 'static,
{
    fn init_handle() -> ExecutorHandles<A, S> {
        single_threaded::divisible_state_exec::DivisibleStateExecutor::<S, A, NT>::init_handle()
    }

    fn init(
        work_receiver: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<DVStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        single_threaded::divisible_state_exec::DivisibleStateExecutor::<S, A, NT>::init::<
            ReplicaReplier,
        >(work_receiver, initial_state, service, send_node)
    }
}

impl<A, S, NT> TDivisibleStateExecutor<A, S, NT> for MultiThreadedDivExecutor
where
    A: ScalableApp<S> + Send + 'static,
    S: DivisibleState + CRUDState + Send + Sync + 'static,
    NT: 'static,
{
    fn init_handle() -> ExecutorHandles<A, S> {
        scalable::divisible_state_exec::ScalableDivisibleStateExecutor::<S, A, NT>::init_handle()
    }

    fn init(
        work_receiver: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<DVStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        scalable::divisible_state_exec::ScalableDivisibleStateExecutor::<S, A, NT>::init::<
            ReplicaReplier,
        >(work_receiver, initial_state, service, send_node)
    }
}

impl<A, S, NT> TMonolithicStateExecutor<A, S, NT> for SingleThreadedMonExecutor
where
    A: Application<S> + 'static,
    S: MonolithicState + 'static,
    NT: 'static,
{
    fn init_handle() -> ExecutorHandles<A, S> {
        single_threaded::monolithic_executor::MonolithicExecutor::<S, A, NT>::init_handle()
    }

    fn init(
        work_receiver: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<MonStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        single_threaded::monolithic_executor::MonolithicExecutor::<S, A, NT>::init::<ReplicaReplier>(
            work_receiver,
            initial_state,
            service,
            send_node,
        )
    }
}

impl<A, S, NT> TMonolithicStateExecutor<A, S, NT> for MultiThreadedMonExecutor
where
    A: ScalableApp<S> + 'static,
    S: MonolithicState + CRUDState + Send + Sync + 'static,
    NT: 'static,
{
    fn init_handle() -> ExecutorHandles<A, S> {
        scalable::monolithic_exec::ScalableMonolithicExecutor::<S, A, NT>::init_handle()
    }

    fn init(
        work_receiver: ChannelSyncRx<ExecutionRequest<Request<A, S>>>,
        initial_state: Option<(S, Vec<Request<A, S>>)>,
        service: A,
        send_node: Arc<NT>,
    ) -> Result<MonStateInstallHandle<S>>
    where
        NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    {
        scalable::monolithic_exec::ScalableMonolithicExecutor::<S, A, NT>::init::<ReplicaReplier>(
            work_receiver,
            initial_state,
            service,
            send_node,
        )
    }
}

pub trait ExecutorReplier: Send {
    fn execution_finished<D, NT>(node: Arc<NT>, seq: Option<SeqNo>, batch: BatchReplies<D::Reply>)
    where
        D: ApplicationData + 'static,
        NT: ReplyNode<SMRReply<D>> + 'static;
}

pub struct FollowerReplier;

impl ExecutorReplier for FollowerReplier {
    fn execution_finished<D, NT>(node: Arc<NT>, seq: Option<SeqNo>, batch: BatchReplies<D::Reply>)
    where
        D: ApplicationData + 'static,
        NT: ReplyNode<SMRReply<D>> + 'static,
    {
        if seq.is_none() {
            //Followers only deliver replies to the unordered requests, since it's not part of the quorum
            // And the requests it executes are only forwarded to it

            ReplicaReplier::execution_finished::<D, NT>(node, seq, batch);
        }
    }
}

pub struct ReplicaReplier;

impl ExecutorReplier for ReplicaReplier {
    fn execution_finished<D, NT>(
        send_node: Arc<NT>,
        seq: Option<SeqNo>,
        batch: BatchReplies<D::Reply>,
    ) where
        D: ApplicationData + 'static,
        NT: ReplyNode<SMRReply<D>> + 'static,
    {
        if batch.is_empty() {
            //Ignore empty batches.
            return;
        }

        let start = Instant::now();

        let batch_type = if seq.is_some() {
            RequestType::Ordered
        } else {
            RequestType::Unordered
        };

        threadpool::execute(move || {
            let batch = batch.into_inner();

            //batch.sort_unstable_by_key(|update_reply| update_reply.to());

            // keep track of the last message and node id
            // we iterated over
            let mut curr_send = None;

            for update_reply in batch {
                let (peer_id, session_id, operation_id, payload) = update_reply.into_inner();

                metric_correlation_id_ended(
                    RQ_CLIENT_TRACKING_ID,
                    create_rq_correlation_id_from_parts(peer_id, session_id, operation_id),
                    REPLYING_TO_REQUEST.clone(),
                );

                metric_correlation_time_end(
                    RQ_CLIENT_TRACK_GLOBAL_ID,
                    create_rq_correlation_id_from_parts(peer_id, session_id, operation_id),
                );

                // NOTE: the technique used here to peek the next reply is a
                // hack... when we port this fix over to the production
                // branch, perhaps we can come up with a better approach,
                // but for now this will do
                if let Some((message, last_peer_id)) = curr_send.take() {
                    let flush = peer_id != last_peer_id;

                    if let Err(err) =
                        send_node.send_signed(batch_type, message, last_peer_id, flush)
                    {
                        error!("Failed to send reply to node {:?} {:?}", peer_id, err);
                    }
                }

                // store previous reply message and peer id,
                // for the next iteration
                //TODO: Choose ordered or unordered reply
                let message = ReplyMessage::new(session_id, operation_id, payload);

                curr_send = Some((message, peer_id));
            }

            // deliver last reply
            if let Some((message, last_peer_id)) = curr_send {
                if let Err(err) = send_node.send_signed(batch_type, message, last_peer_id, true) {
                    error!("Failed to send reply to node {:?} {:?}", last_peer_id, err);
                }
            } else {
                // slightly optimize code path;
                // the previous if branch will always execute
                // (there is always at least one request in the batch)
                unreachable!();
            }

            metric_duration(REPLIES_SENT_TIME_ID, start.elapsed());
        });
    }
}
