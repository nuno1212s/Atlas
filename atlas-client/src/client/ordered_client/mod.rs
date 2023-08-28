use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_execution::serialize::ApplicationData;
use atlas_core::messages::{RequestMessage, SystemMessage};
use atlas_core::reconfiguration_protocol::ReconfigurationProtocol;
use atlas_core::serialize::ClientMessage;
use super::{ClientType, Client};

pub struct Ordered;

impl<RF, D, NT> ClientType<RF, D, NT> for Ordered
    where D: ApplicationData + 'static,
          RF: ReconfigurationProtocol + 'static, {
    fn init_request(
        session_id: SeqNo,
        operation_id: SeqNo,
        operation: D::Request,
    ) -> ClientMessage<D> {
        SystemMessage::OrderedRequest(RequestMessage::new(session_id, operation_id, operation))
    }

    type Iter = impl Iterator<Item=NodeId>;

    fn init_targets(client: &Client<RF, D, NT>) -> (Self::Iter, usize) {
        let quorum = client.get_quorum_view();

        let quorum_len = quorum.len();

        (quorum.into_iter(), quorum_len)
    }

    fn needed_responses(client: &Client<RF, D, NT>) -> usize {
        client.params.f() + 1
    }
}