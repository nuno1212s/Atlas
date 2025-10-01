use crate::byte_stub::connections::NetworkConnectionController;
use crate::byte_stub::stub_endpoint::StubEndpoint;
use crate::byte_stub::ByteNetworkStub;
use crate::reconfiguration::NetworkInformationProvider;
use crate::serialization::Serializable;
use crate::stub::{
    ApplicationStub, BatchedNetworkStub, OperationStub, ReconfigurationStub, RegularNetworkStub,
    StateProtocolStub,
};

/// The regular network stub implementation for the reconfiguration stub
impl<NI, CN, BNC, R, O, S, A> RegularNetworkStub<R> for ReconfigurationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    type Incoming = StubEndpoint<R::Message>;

    fn incoming_stub(&self) -> &Self::Incoming {
        &self.stub_endpoint
    }
}

/// The regular network stub implementation for the operation stub
impl<NI, CN, BNC, R, O, S, A> RegularNetworkStub<O> for OperationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    type Incoming = StubEndpoint<O::Message>;

    fn incoming_stub(&self) -> &Self::Incoming {
        &self.stub_endpoint
    }
}

impl<NI, CN, BNC, R, O, S, A> RegularNetworkStub<S> for StateProtocolStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    type Incoming = StubEndpoint<S::Message>;

    fn incoming_stub(&self) -> &Self::Incoming {
        &self.stub_endpoint
    }
}

/// The application stub is special
/// Application stubs can either be regular stubs (if we are a client)
/// or pooled, batched stubs (if we are a replica)
/// Therefore, we must have both implementations since they are both valid
impl<NI, CN, BNC, R, O, S, A> RegularNetworkStub<A> for ApplicationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    type Incoming = StubEndpoint<A::Message>;

    fn incoming_stub(&self) -> &Self::Incoming {
        &self.stub_endpoint
    }
}

impl<NI, CN, BNC, R, O, S, A> BatchedNetworkStub<A> for ApplicationStub<NI, CN, BNC, R, O, S, A>
where
    R: Serializable + 'static,
    O: Serializable + 'static,
    S: Serializable + 'static,
    A: Serializable + 'static,
    NI: NetworkInformationProvider,
    CN: ByteNetworkStub + 'static,
    BNC: NetworkConnectionController,
{
    type Incoming = StubEndpoint<A::Message>;

    fn incoming_stub(&self) -> &Self::Incoming {
        &self.stub_endpoint
    }
}
