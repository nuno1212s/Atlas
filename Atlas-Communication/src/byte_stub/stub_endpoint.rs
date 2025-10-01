use crate::byte_stub::incoming::{pooled_stub, unpooled_stub};
use crate::message::StoredMessage;
use crate::stub::{BatchedModuleIncomingStub, ModuleIncomingStub};
use atlas_common::channel::sync::ChannelSyncRx;
use std::time::Duration;

/// The output stub for a given message module
///
#[derive(Clone)]
pub enum StubEndpoint<M>
where
    M: Send,
{
    Unpooled(unpooled_stub::UnpooledStubRX<StoredMessage<M>>),
    Pooled(pooled_stub::PooledStubOutput<StoredMessage<M>>),
}

impl<M> AsRef<ChannelSyncRx<StoredMessage<M>>> for StubEndpoint<M>
where
    M: Send + Clone,
{
    fn as_ref(&self) -> &ChannelSyncRx<StoredMessage<M>> {
        match self {
            StubEndpoint::Unpooled(unpooled_stub) => unpooled_stub.as_ref(),
            _ => unreachable!(),
        }
    }
}

impl<M> AsRef<ChannelSyncRx<Vec<StoredMessage<M>>>> for StubEndpoint<M>
where
    M: Send + Clone,
{
    fn as_ref(&self) -> &ChannelSyncRx<Vec<StoredMessage<M>>> {
        match self {
            StubEndpoint::Pooled(pooled_stub) => pooled_stub.as_ref(),
            _ => unreachable!(),
        }
    }
}

impl<M> ModuleIncomingStub<M> for StubEndpoint<M>
where
    M: Send + Clone,
{
    fn pending_rqs(&self) -> usize {
        match self {
            StubEndpoint::Unpooled(unpooled_stub) => unpooled_stub.pending_rqs(),
            _ => unreachable!(),
        }
    }

    fn receive_messages(&self) -> atlas_common::error::Result<StoredMessage<M>> {
        match self {
            StubEndpoint::Unpooled(unpooled_stub) => unpooled_stub.receive_messages(),
            _ => unreachable!(),
        }
    }

    fn try_receive_messages(
        &self,
        timeout: Option<Duration>,
    ) -> atlas_common::error::Result<Option<StoredMessage<M>>> {
        match self {
            StubEndpoint::Unpooled(unpooled_stub) => unpooled_stub.try_receive_messages(timeout),
            _ => unreachable!(),
        }
    }
}

impl<M> BatchedModuleIncomingStub<M> for StubEndpoint<M>
where
    M: Send + Clone,
{
    fn receive_messages(&self) -> atlas_common::error::Result<Vec<StoredMessage<M>>> {
        match &self {
            StubEndpoint::Pooled(pooled) => pooled.receive_messages(),
            _ => unreachable!(),
        }
    }

    fn try_receive_messages(
        &self,
        timeout: Option<Duration>,
    ) -> atlas_common::error::Result<Option<Vec<StoredMessage<M>>>> {
        match &self {
            StubEndpoint::Pooled(pooled) => pooled.try_receive_messages(timeout),
            _ => unreachable!(),
        }
    }
}
