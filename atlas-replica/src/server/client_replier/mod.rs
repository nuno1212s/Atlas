use std::ops::Deref;
use std::sync::Arc;

use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::node_id::NodeId;
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_core::messages::{ReplyMessage, SystemMessage};
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_core::serialize::Service;
use atlas_core::state_transfer::networking::serialize::StateTransferMessage;
use atlas_execution::app::BatchReplies;
use atlas_execution::serialize::ApplicationData;

type RepliesType<S> = BatchReplies<S>;

///Dedicated thread to reply to clients
/// This is currently not being used (we are currently using the thread pool)
pub struct Replier<D, NT> where D: ApplicationData + 'static {
    node_id: NodeId,
    channel: ChannelSyncRx<RepliesType<D::Reply>>,
    send_node: Arc<NT>,
}

pub struct ReplyHandle<D> where D: ApplicationData {
    inner: ChannelSyncTx<RepliesType<D::Reply>>,
}

const REPLY_CHANNEL_SIZE: usize = 1024;

impl<D> ReplyHandle<D> where D: ApplicationData {
    pub fn new(replier: ChannelSyncTx<RepliesType<D::Reply>>) -> Self {
        Self {
            inner: replier
        }
    }
}

impl<D> Deref for ReplyHandle<D> where D: ApplicationData {
    type Target = ChannelSyncTx<RepliesType<D::Reply>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<D> Clone for ReplyHandle<D> where D: ApplicationData {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone()
        }
    }
}

impl<D, NT: 'static> Replier<D, NT> where D: ApplicationData + 'static {
    pub fn new(node_id: NodeId, send_node: Arc<NT>) -> ReplyHandle<D> {
        let (ch_tx, ch_rx) = channel::new_bounded_sync(REPLY_CHANNEL_SIZE);

        let reply_task = Self {
            node_id,
            channel: ch_rx,
            send_node,
        };

        let handle = ReplyHandle::new(ch_tx);

        //reply_task.start();

        handle
    }

    pub fn start<OP, ST, LP>(mut self) where OP: OrderingProtocolMessage<D> + 'static,
                                             ST: StateTransferMessage + 'static,
                                             LP: LogTransferMessage<D, OP> + 'static,
                                             NT: ProtocolNetworkNode<Service<D, OP, ST, LP>> {
        std::thread::Builder::new().name(format!("{:?} // Reply thread", self.node_id))
            .spawn(move || {
                loop {
                    let reply_batch = self.channel.recv().unwrap();

                    let mut batch = reply_batch.into_inner();

                    batch.sort_unstable_by_key(|update_reply| update_reply.to());

                    // keep track of the last message and node id
                    // we iterated over
                    let mut curr_send = None;

                    for update_reply in batch {
                        let (peer_id, session_id, operation_id, payload) = update_reply.into_inner();

                        // NOTE: the technique used here to peek the next reply is a
                        // hack... when we port this fix over to the production
                        // branch, perhaps we can come up with a better approach,
                        // but for now this will do
                        if let Some((message, last_peer_id)) = curr_send.take() {
                            let flush = peer_id != last_peer_id;
                            self.send_node.send(message, last_peer_id, flush);
                        }

                        // store previous reply message and peer id,
                        // for the next iteration
                        //TODO: Choose ordered or unordered reply
                        let message = SystemMessage::OrderedReply(ReplyMessage::new(
                            session_id,
                            operation_id,
                            payload,
                        ));

                        curr_send = Some((message, peer_id));
                    }

                    // deliver last reply
                    if let Some((message, last_peer_id)) = curr_send {
                        self.send_node.send(message, last_peer_id, true);
                    } else {
                        // slightly optimize code path;
                        // the previous if branch will always execute
                        // (there is always at least one request in the batch)
                        unreachable!();
                    }
                }
            }).expect("Failed to launch thread for client replier!");
    }
}