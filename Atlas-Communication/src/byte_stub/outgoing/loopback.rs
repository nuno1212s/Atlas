use crate::byte_stub::incoming;
use crate::lookup_table::{MessageModule, ModMessageWrapped, PeerStubLookupTable};
use crate::message::{Header, StoredMessage};
use crate::serialization::Serializable;

pub struct LoopbackOutgoingStub<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    loopback_stubs: incoming::PeerStubLookupTable<R, O, S, A>,
}

impl<R, O, S, A> LoopbackOutgoingStub<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    pub fn init(stub_controller: incoming::PeerStubLookupTable<R, O, S, A>) -> Self {
        Self {
            loopback_stubs: stub_controller,
        }
    }

    pub fn handle_message(&self, header: Header, message: ModMessageWrapped<R, O, S, A>) {
        match message {
            ModMessageWrapped::Reconfiguration(r) => {
                self.handle_reconf_message(StoredMessage::new(header, r));
            }
            ModMessageWrapped::Protocol(o) => {
                self.handle_protocol_message(StoredMessage::new(header, o));
            }
            ModMessageWrapped::StateProtocol(s) => {
                self.handle_state_protocol_message(StoredMessage::new(header, s));
            }
            ModMessageWrapped::Application(a) => {
                self.handle_application_message(StoredMessage::new(header, a));
            }
        }
    }

    fn handle_reconf_message(&self, message: StoredMessage<R::Message>) {
        let stub = self
            .loopback_stubs
            .get_stub_for_message(&MessageModule::Reconfiguration);

        let (header, message) = message.into_inner();

        stub.push_reconfiguration(header, message).unwrap();
    }

    // Handle the rest of the message modules
    fn handle_protocol_message(&self, message: StoredMessage<O::Message>) {
        let stub = self
            .loopback_stubs
            .get_stub_for_message(&MessageModule::Protocol);

        let (header, message) = message.into_inner();

        stub.push_protocol(header, message).unwrap();
    }

    fn handle_state_protocol_message(&self, message: StoredMessage<S::Message>) {
        let stub = self
            .loopback_stubs
            .get_stub_for_message(&MessageModule::StateProtocol);

        let (header, message) = message.into_inner();

        stub.push_state_protocol(header, message).unwrap();
    }

    fn handle_application_message(&self, message: StoredMessage<A::Message>) {
        let stub = self
            .loopback_stubs
            .get_stub_for_message(&MessageModule::Application);

        let (header, message) = message.into_inner();

        stub.push_application(header, message).unwrap();
    }
}

impl<R, O, S, A> Clone for LoopbackOutgoingStub<R, O, S, A>
where
    R: Serializable,
    O: Serializable,
    S: Serializable,
    A: Serializable,
{
    fn clone(&self) -> Self {
        Self {
            loopback_stubs: self.loopback_stubs.clone(),
        }
    }
}
