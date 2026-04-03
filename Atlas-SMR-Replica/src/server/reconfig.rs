use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::message::StoredMessage;
use atlas_core::ordering_protocol::OrderingProtocol;
use atlas_core::ordering_protocol::networking::serialize::NetworkView;
use atlas_core::ordering_protocol::permissioned::{VTMsg, ViewTransferProtocol};
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_core::SMRReq;

/// This is used to keep track of the node that is currently
/// attempting to join the server
#[derive(Default, Debug)]
pub(super) struct QuorumReconfig {
    node_pending_join: Option<NodeId>,
}

impl QuorumReconfig {
    /// Attempt to register the node
    pub(super) fn append_pending_node_join(&mut self, node: NodeId) -> bool {
        if self.node_pending_join.is_none() {
            self.node_pending_join = Some(node);
            true
        } else {
            false
        }
    }

    pub(super) fn pop_pending_node_join(&mut self) -> Option<NodeId> {
        self.node_pending_join.take()
    }
}

pub enum IterableProtocolRes {
    ReRun,
    Receive,
    Continue,
}

/// The trait with methods specific to reconfigurable protocol handle
/// This is then combined with specialization in order to maintain
/// optional support for this type of protocols
pub trait ReconfigurableProtocolHandling {
    fn attempt_quorum_join(&mut self, node: NodeId) -> atlas_common::error::Result<()>;

    fn attempt_to_join_quorum(&mut self) -> atlas_common::error::Result<()>;
}

/// Trait with methods specific to reconfigurable protocol handle
/// This is then combined with specialization in order to provide
/// optional support for this type of protocols
pub(crate) trait PermissionedProtocolHandling<D, VT, OP, NT>
where
    OP: OrderingProtocol<SMRReq<D>>,
    VT: ViewTransferProtocol<OP>,
    D: ApplicationData,
{
    type View: NetworkView + 'static;

    fn view(&self) -> Self::View;

    fn run_view_transfer(&mut self) -> atlas_common::error::Result<()>;

    fn iterate_view_transfer_protocol(
        &mut self,
    ) -> atlas_common::error::Result<IterableProtocolRes>;

    fn handle_view_transfer_msg(
        &mut self,
        msg: StoredMessage<VTMsg<VT::Serialization>>,
    ) -> atlas_common::error::Result<()>;
}

#[derive(Clone, Debug)]
pub struct MockView(Vec<NodeId>);

impl Orderable for MockView {
    fn sequence_number(&self) -> SeqNo {
        SeqNo::ZERO
    }
}

impl NetworkView for MockView {
    fn primary(&self) -> NodeId {
        self.0[0]
    }

    fn quorum(&self) -> usize {
        (self.n() - 1) / 2
    }

    fn quorum_members(&self) -> &Vec<NodeId> {
        &self.0
    }

    fn f(&self) -> usize {
        (self.n() - 1) / 3
    }

    fn n(&self) -> usize {
        self.0.len()
    }
}
