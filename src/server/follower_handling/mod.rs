use std::sync::Arc;

use atlas_common::channel;
use atlas_common::channel::sync::ChannelSyncRx;
use atlas_common::globals::ReadOnly;
use atlas_common::node_id::NodeId;
use atlas_communication::message::StoredMessage;
use atlas_communication::stub::{ModuleOutgoingStub, RegularNetworkStub};
use atlas_core::followers::{FollowerChannelMsg, FollowerEvent, FollowerHandle};
use atlas_core::messages::Protocol;
use atlas_core::ordering_protocol::networking::serialize::{
    NetworkView, OrderingProtocolMessage, PermissionedOrderingProtocolMessage,
    ViewTransferProtocolMessage,
};
use atlas_logging_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_core::message::SystemMessage;
use atlas_smr_core::serialize::Service;
use atlas_smr_core::state_transfer::networking::serialize::StateTransferMessage;
use atlas_smr_core::SMRReq;

/// Store information of the current followers of the quorum
/// This information will be used to calculate which replicas have to send the
/// Information to what followers
///
/// This routing is only relevant to the Preprepare requests, all other requests
/// Can be broadcast from each replica as they are very small and therefore
/// don't have any effects on performance
struct FollowersFollowing<
    D: ApplicationData,
    OP: OrderingProtocolMessage<SMRReq<D>>,
    POP: PermissionedOrderingProtocolMessage,
    NT,
> {
    own_id: NodeId,
    followers: Vec<NodeId>,
    send_node: Arc<NT>,
    rx: ChannelSyncRx<FollowerChannelMsg<SMRReq<D>, OP, POP>>,
}

impl<D, OP, POP, NT> FollowersFollowing<D, OP, POP, NT>
where
    D: ApplicationData + 'static,
    OP: OrderingProtocolMessage<SMRReq<D>> + 'static,
    POP: PermissionedOrderingProtocolMessage + 'static,
    NT: Send + Sync + 'static,
{
    /// Starts the follower handling thread and returns a cloneable handle that
    /// can be used to deliver messages to it.
    pub fn init_follower_handling<ST, LP, VT>(
        id: NodeId,
        node: &Arc<NT>,
    ) -> FollowerHandle<SMRReq<D>, OP, POP>
    where
        D: ApplicationData + 'static,
        ST: StateTransferMessage + 'static,
        LP: LogTransferMessage<SMRReq<D>, OP> + 'static,
        VT: ViewTransferProtocolMessage + 'static,
        NT: RegularNetworkStub<Service<D, OP, LP, VT>>,
    {
        let (tx, rx) = channel::sync::new_bounded_sync(1024, Some("Follower Channel"));

        let follower_handling = Self {
            own_id: id,
            followers: Vec::new(),
            send_node: Arc::clone(node),
            rx,
        };

        Self::start_thread::<ST, LP, VT>(follower_handling);

        FollowerHandle::new(tx)
    }

    fn start_thread<ST, LP, VT>(self)
    where
        D: ApplicationData + 'static,
        ST: StateTransferMessage + 'static,
        LP: LogTransferMessage<SMRReq<D>, OP> + 'static,
        VT: ViewTransferProtocolMessage + 'static,
        NT: RegularNetworkStub<Service<D, OP, LP, VT>>,
    {
        std::thread::Builder::new()
            .name(format!(
                "Follower Handling Thread for node {:?}",
                self.own_id
            ))
            .spawn(move || {
                self.run::<ST, LP, VT>();
            })
            .expect("Failed to launch follower handling thread!");
    }

    fn run<ST, LP, VT>(mut self)
    where
        D: ApplicationData + 'static,
        ST: StateTransferMessage + 'static,
        LP: LogTransferMessage<SMRReq<D>, OP> + 'static,
        VT: ViewTransferProtocolMessage + 'static,
        NT: RegularNetworkStub<Service<D, OP, LP, VT>>,
    {
        loop {
            let message = self.rx.recv().unwrap();

            match message {
                FollowerEvent::ReceivedConsensusMsg(view, consensus_msg) => {
                    todo!()
                }
                FollowerEvent::ReceivedViewChangeMsg(view_change_msg) => {
                    self.handle_sync_msg::<ST, LP, VT>(view_change_msg)
                }
            }
        }
    }

    /// Calculate which followers we have to send the messages to
    /// according to the disposition of the quorum and followers
    ///
    /// (This is only needed for the preprepare message, all others use
    /// multicast)
    fn targets(&self, view: &POP::ViewInfo) -> Vec<NodeId> {
        //How many replicas are not the leader?
        let available_replicas = view.n() - 1;

        //How many followers do we have to provide for
        let followers = self.followers.len();

        //We only need one pre prepare in reality, since it is signed by the current leader
        //And can't be forged, but since we want to prevent message dropping attacks,
        //We need to use f+1 replicas
        let replicas_per_follower = view.f() + 1;

        //We do not want to have spaces between each id so we don't get inconsistencies
        //In how we arrange the replicas
        //In this layout, we will always get 0, 1, 2 as IDs, independently of what the leader
        //is
        let temp_id = if self.own_id > view.primary() {
            NodeId::from(self.own_id.id() - 1)
        } else {
            self.own_id
        };

        if followers >= available_replicas {
            //How many followers do we have to forward the message to
            //Taking all of this into account
            let followers_for_replica = (replicas_per_follower * followers) / available_replicas;

            let first_follower = temp_id.id() % (self.followers.len() as u32);

            let last_follower = first_follower + followers_for_replica as u32;

            let mut targetted_followers =
                Vec::with_capacity((last_follower - first_follower) as usize);

            for i in first_follower..=last_follower {
                targetted_followers.push(self.followers[i as usize]);
            }

            targetted_followers
        } else {
            //TODO: How to handle layouts when there are more replicas than followers?
            todo!()
        }
    }

    /// Handle when we have received a preprepare message
    fn handle_preprepare_msg_rcvd<ST, LP, VT>(
        &mut self,
        view: &POP::ViewInfo,
        message: Arc<ReadOnly<StoredMessage<Protocol<OP::ProtocolMessage>>>>,
    ) where
        D: ApplicationData + 'static,
        ST: StateTransferMessage + 'static,
        LP: LogTransferMessage<SMRReq<D>, OP> + 'static,
        VT: ViewTransferProtocolMessage + 'static,
        NT: RegularNetworkStub<Service<D, OP, LP, VT>>,
    {
        if view.primary() == self.own_id {
            //Leaders don't send pre_prepares to followers in order to save bandwidth
            //as they already have to send the to all of the replicas
            return;
        }

        //Clone the messages here in this thread so we don't slow down the consensus thread at all
        let header = message.header().clone();

        let pre_prepare = message.message().clone();

        let message =
            SystemMessage::from_fwd_protocol_message(StoredMessage::new(header, pre_prepare));

        let targets = self.targets(view);

        let _ = self
            .send_node
            .outgoing_stub()
            .broadcast(message, targets.into_iter());
    }

    /// Handle us having sent a prepare message (notice how pre prepare are handled on reception
    /// and prepare/commit are handled on sending, this is because we don't want the leader
    /// to have to send the pre prepare to all followers but since these messages are very small,
    /// it's fine for all replicas to broadcast it to followers)
    fn handle_prepare_msg<ST, LP, VT>(
        &mut self,
        prepare: Arc<ReadOnly<StoredMessage<Protocol<OP::ProtocolMessage>>>>,
    ) where
        D: ApplicationData + 'static,
        ST: StateTransferMessage + 'static,
        LP: LogTransferMessage<SMRReq<D>, OP> + 'static,
        VT: ViewTransferProtocolMessage + 'static,
        NT: RegularNetworkStub<Service<D, OP, LP, VT>>,
    {
        if prepare.header().from() != self.own_id {
            //We only broadcast our own prepare messages, not other peoples
            return;
        }

        let header = prepare.header().clone();

        //Clone the messages here in this thread so we don't slow down the consensus thread at all
        let prepare = prepare.message().clone();

        let message = SystemMessage::from_fwd_protocol_message(StoredMessage::new(header, prepare));

        let _ = self
            .send_node
            .outgoing_stub()
            .broadcast(message, self.followers.iter().copied());
    }

    /// Handle us having sent a commit message (notice how pre prepare are handled on reception
    /// and prepare/commit are handled on sending, this is because we don't want the leader
    /// to have to send the pre prepare to all followers but since these messages are very small,
    /// it's fine for all replicas to broadcast it to followers)
    fn handle_commit_msg<ST, LP, VT>(
        &mut self,
        commit: Arc<ReadOnly<StoredMessage<Protocol<OP::ProtocolMessage>>>>,
    ) where
        D: ApplicationData + 'static,
        ST: StateTransferMessage + 'static,
        LP: LogTransferMessage<SMRReq<D>, OP> + 'static,
        VT: ViewTransferProtocolMessage + 'static,
        NT: RegularNetworkStub<Service<D, OP, LP, VT>>,
    {
        if commit.header().from() != self.own_id {
            //Like with prepares, we only broadcast our own commit messages
            return;
        }

        let header = commit.header().clone();
        let commit = commit.message().clone();

        let message = SystemMessage::from_fwd_protocol_message(StoredMessage::new(header, commit));

        let _ = self
            .send_node
            .outgoing_stub()
            .broadcast(message, self.followers.iter().copied());
    }

    ///
    fn handle_sync_msg<ST, LP, VT>(
        &mut self,
        msg: Arc<ReadOnly<StoredMessage<Protocol<OP::ProtocolMessage>>>>,
    ) where
        D: ApplicationData + 'static,
        ST: StateTransferMessage + 'static,
        LP: LogTransferMessage<SMRReq<D>, OP> + 'static,
        VT: ViewTransferProtocolMessage + 'static,
        NT: RegularNetworkStub<Service<D, OP, LP, VT>>,
    {
        let header = msg.header().clone();
        let message = msg.message().clone();

        let network_msg =
            SystemMessage::from_fwd_protocol_message(StoredMessage::new(header, message));

        let _ = self
            .send_node
            .outgoing_stub()
            .broadcast(network_msg, self.followers.iter().copied());
    }
}
