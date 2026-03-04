use std::sync::Arc;
use std::time::Instant;
use atlas_common::ordering::SeqNo;
use atlas_common::threadpool;
use atlas_core::execution::requests::{ReplyBatch, UpdateInfo};
use atlas_core::messages::{create_rq_correlation_id_from_parts, ReplyMessage};
use atlas_core::metric::{RQ_CLIENT_TRACKING_ID, RQ_CLIENT_TRACK_GLOBAL_ID};
use atlas_metrics::metrics::{metric_correlation_id_ended, metric_correlation_time_end, metric_duration};
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_core::execution::reply::{ReplyNode, RequestType};
use atlas_smr_core::SMRReply;
use tracing::error;
use crate::metric::{REPLIES_SENT_TIME_ID, REPLYING_TO_REQUEST};

pub trait ExecutorReplier: Send {
    fn execution_finished<D, NT>(node: Arc<NT>, seq: Option<SeqNo>, batch: ReplyBatch<D::Reply>)
    where
        D: ApplicationData + 'static,
        NT: ReplyNode<SMRReply<D>> + 'static;
}

pub struct FollowerReplier;

impl ExecutorReplier for FollowerReplier {
    fn execution_finished<D, NT>(node: Arc<NT>, seq: Option<SeqNo>, batch: ReplyBatch<D::Reply>)
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
        batch: ReplyBatch<D::Reply>,
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
                let (info, payload) = update_reply.into_inner();

                let UpdateInfo::SessionBased {
                    from,
                    session_number,
                    sequence_number,
                } = info;

                metric_correlation_id_ended(
                    RQ_CLIENT_TRACKING_ID,
                    create_rq_correlation_id_from_parts(from, session_number, sequence_number),
                    REPLYING_TO_REQUEST.clone(),
                );

                metric_correlation_time_end(
                    RQ_CLIENT_TRACK_GLOBAL_ID,
                    create_rq_correlation_id_from_parts(from, session_number, sequence_number),
                );

                // NOTE: the technique used here to peek the next reply is a
                // hack... when we port this fix over to the production
                // branch, perhaps we can come up with a better approach,
                // but for now this will do
                if let Some((message, last_peer_id)) = curr_send.take() {
                    let flush = from != last_peer_id;

                    if let Err(err) =
                        send_node.send_signed(batch_type, message, last_peer_id, flush)
                    {
                        error!("Failed to send reply to node {:?} {:?}", from, err);
                    }
                }

                // store previous reply message and peer id,
                // for the next iteration
                //TODO: Choose ordered or unordered reply
                let message = ReplyMessage::new(session_number, sequence_number, payload);

                curr_send = Some((message, from));
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