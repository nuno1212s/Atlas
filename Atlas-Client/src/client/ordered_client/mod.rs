use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_core::messages::RequestMessage;
use atlas_core::reconfiguration_protocol::ReconfigurationProtocol;
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_core::message::OrderableMessage;
use atlas_smr_core::serialize::SMRSysMessage;

use super::{Client, ClientType};

pub struct Ordered;

impl<RF, D, NT> ClientType<RF, D, NT> for Ordered
where
    D: ApplicationData + 'static,
    RF: ReconfigurationProtocol + 'static,
{
    fn init_request(
        session_id: SeqNo,
        operation_id: SeqNo,
        operation: D::Request,
    ) -> SMRSysMessage<D> {
        OrderableMessage::OrderedRequest(RequestMessage::new(session_id, operation_id, operation))
    }

    type Iter = impl Iterator<Item = NodeId>;

    fn init_targets(client: &Client<RF, D, NT>) -> (Self::Iter, usize) {
        let quorum = client.get_quorum_view();

        let quorum_len = quorum.len();

        (quorum.into_iter(), quorum_len)
    }

    fn needed_responses(client: &Client<RF, D, NT>) -> usize {
        client.data.reconfig_protocol.get_current_f() + 1
    }
}
