use std::ops::Deref;
use std::time::Duration;

use crate::config::UnpooledConnection;
use crate::lookup_table::MessageModule;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::channel::TryRecvError;
use atlas_common::node_id::NodeId;
use atlas_common::{channel, Err};

use crate::message::StoredMessage;
use crate::stub::ModuleIncomingStub;

/// The stub controller, responsible for generating and managing stubs for connected peers.
///
/// In the case of unpooled stubs, we don't need to maintain a connection map, since we don't
/// distinguish where the message came from. They all end up in the same channel.
pub struct UnpooledStubManagement<M> {
    tx: ChannelSyncTx<M>,
}

impl<M> UnpooledStubManagement<M> {
    pub fn initialize_controller(
        _config: UnpooledConnection,
        module: MessageModule,
    ) -> (Self, UnpooledStubRX<M>) {
        //TODO: Bounded sync
        let (tx, rx) = channel::sync::new_unbounded_sync(Some(format!(
            "Unpooled stub {module:?} (Ingestion)"
        )));

        (Self { tx }, UnpooledStubRX { rx })
    }

    pub fn gen_stub_stub_for_peer(&self, _peer: NodeId) -> UnpooledStubTX<M> {
        UnpooledStubTX {
            tx: self.tx.clone(),
        }
    }
}

/// An unpooled stub, meant for connections with not a lot of throughput but
/// very latency sensitive
pub struct UnpooledStubRX<M> {
    rx: ChannelSyncRx<M>,
}

/// The send side of this stub type
pub struct UnpooledStubTX<M> {
    tx: ChannelSyncTx<M>,
}

impl<M> Deref for UnpooledStubTX<M> {
    type Target = ChannelSyncTx<M>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl<M> Clone for UnpooledStubTX<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<M> Clone for UnpooledStubRX<M> {
    fn clone(&self) -> Self {
        Self {
            rx: self.rx.clone(),
        }
    }
}

impl<T> AsRef<ChannelSyncRx<StoredMessage<T>>> for UnpooledStubRX<StoredMessage<T>>
where
    T: Send,
{
    fn as_ref(&self) -> &ChannelSyncRx<StoredMessage<T>> {
        &self.rx
    }
}

impl<T> ModuleIncomingStub<T> for UnpooledStubRX<StoredMessage<T>>
where
    T: Send,
{
    fn pending_rqs(&self) -> usize {
        self.rx.len()
    }

    fn receive_messages(&self) -> atlas_common::error::Result<StoredMessage<T>> {
        self.rx.recv().map_err(|err| err.into())
    }

    fn try_receive_messages(
        &self,
        timeout: Option<Duration>,
    ) -> atlas_common::error::Result<Option<StoredMessage<T>>> {
        let result = if let Some(timeout) = timeout {
            self.rx.recv_timeout(timeout)
        } else {
            self.rx.try_recv()
        };

        match result {
            Ok(message) => Ok(Some(message)),
            Err(err) => match err {
                TryRecvError::ChannelDc => {
                    Err!(TryRecvError::ChannelDc)
                }
                TryRecvError::ChannelEmpty | TryRecvError::Timeout => Ok(None),
            },
        }
    }
}
