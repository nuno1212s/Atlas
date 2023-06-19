use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::{Node};
use atlas_execution::serialize::ApplicationData;
use atlas_core::messages::{RequestMessage, SystemMessage};
use atlas_core::serialize::ClientMessage;
use super::{ClientType, Client};

pub struct Ordered;

impl<D, NT> ClientType<D, NT> for Ordered where D: ApplicationData + 'static {
    fn init_request(
        session_id: SeqNo,
        operation_id: SeqNo,
        operation: D::Request,
    ) -> ClientMessage<D> {
        SystemMessage::OrderedRequest(RequestMessage::new(session_id, operation_id, operation))
    }

    type Iter = impl Iterator<Item=NodeId>;

    fn init_targets(client: &Client<D, NT>) -> (Self::Iter, usize) {
        (NodeId::targets(0..client.params.n()), client.params.n())
    }

    fn needed_responses(client: &Client<D, NT>) -> usize {
        client.params.f() + 1
    }
}