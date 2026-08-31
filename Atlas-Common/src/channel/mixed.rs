use crate::channel::{RecvError, SendError, SendReturnError, TryRecvError, TrySendReturnError};
use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

// Async and sync mixed channels (Allows us to connect async and sync
// environments together)

/// Future returned by [`ChannelMixedRx::recv_async`].
///
/// The mixed channel is always backed by flume, whichever backend the async
/// channel group selects, so this wraps the flume future directly instead of
/// reusing `async::ChannelRxFut` (whose inner type follows the async group).
pub struct ChannelRxFut<'a, T> {
    channel: Option<Arc<str>>,
    inner: super::flume_mpmc::ChannelRxFut<'a, T>,
}

/// Future returned by [`ChannelMixedTx::send_async`]. See [`ChannelRxFut`].
pub struct ChannelTxFut<'a, T> {
    channel: Option<Arc<str>>,
    inner: super::flume_mpmc::ChannelTxFut<'a, T>,
}

impl<'a, T> Future for ChannelRxFut<'a, T> {
    type Output = Result<T, RecvError>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let channel = self.channel.clone();

        pin!(&mut self.inner)
            .poll(cx)
            .map(|res| res.map_err(|err| err.with_channel(channel)))
    }
}

impl<'a, T> Future for ChannelTxFut<'a, T> {
    type Output = Result<(), SendError>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let channel = self.channel.clone();

        pin!(&mut self.inner).poll(cx).map(|r| match r {
            Ok(_) => Ok(()),
            Err(_) => Err(SendError::FailedToSend { channel }),
        })
    }
}

impl<'a, T> ChannelRxFut<'a, T> {
    #[inline]
    fn new(inner: super::flume_mpmc::ChannelRxFut<'a, T>, channel: Option<Arc<str>>) -> Self {
        Self { channel, inner }
    }
}

impl<'a, T> ChannelTxFut<'a, T> {
    #[inline]
    fn new(inner: super::flume_mpmc::ChannelTxFut<'a, T>, channel: Option<Arc<str>>) -> Self {
        Self { channel, inner }
    }
}

type InnerChannelMixedRx<T> = super::flume_mpmc::ChannelMixedRx<T>;

type InnerChannelMixedTx<T> = super::flume_mpmc::ChannelMixedTx<T>;

pub struct ChannelMixedRx<T> {
    channel_identifier: Option<Arc<str>>,
    inner: InnerChannelMixedRx<T>,
}

pub struct ChannelMixedTx<T> {
    channel_identifier: Option<Arc<str>>,
    inner: InnerChannelMixedTx<T>,
}

impl<T> ChannelMixedRx<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn recv(&self) -> Result<T, RecvError> {
        self.inner
            .recv_sync()
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    #[inline]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, TryRecvError> {
        self.inner
            .recv_timeout(timeout)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    #[inline]
    pub fn recv_async(&mut self) -> ChannelRxFut<'_, T> {
        ChannelRxFut::new(self.inner.recv(), self.channel_identifier.clone())
    }

    #[inline]
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.inner
            .try_recv()
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }
}

impl<T> ChannelMixedTx<T>
where
    T: 'static,
{
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn send_async(&self, value: T) -> ChannelTxFut<'_, T> {
        ChannelTxFut::new(self.inner.send(value), self.channel_identifier.clone())
    }

    #[inline]
    pub fn send_async_return(&self, value: T) -> ChannelTxFut<'_, T> {
        ChannelTxFut::new(self.inner.send(value), self.channel_identifier.clone())
    }

    #[inline]
    pub fn send(&self, value: T) -> crate::error::Result<()> {
        Ok(self
            .inner
            .send_sync(value)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))?)
    }

    #[inline]
    pub fn send_return(&self, value: T) -> Result<(), SendReturnError<T>> {
        self.inner
            .send_sync_return(value)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    pub fn send_timeout(&self, value: T, timeout: Duration) -> crate::error::Result<()> {
        Ok(self
            .inner
            .send_timeout_sync(value, timeout)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))?)
    }

    #[inline]
    pub fn send_timeout_return(
        &self,
        value: T,
        timeout: Duration,
    ) -> Result<(), TrySendReturnError<T>> {
        self.inner
            .send_timeout_sync_return(value, timeout)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }
}

impl<T> Clone for ChannelMixedTx<T> {
    fn clone(&self) -> Self {
        ChannelMixedTx {
            channel_identifier: self.channel_identifier.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelMixedRx<T> {
    fn clone(&self) -> Self {
        ChannelMixedRx {
            channel_identifier: self.channel_identifier.clone(),
            inner: self.inner.clone(),
        }
    }
}

pub fn new_bounded_mixed<T>(
    bound: usize,
    name: Option<impl Into<String>>,
) -> (ChannelMixedTx<T>, ChannelMixedRx<T>) {
    let name = name.map(|string| Arc::from(string.into()));

    let (tx, rx) = super::flume_mpmc::new_bounded(bound);

    (
        ChannelMixedTx {
            channel_identifier: name.clone(),
            inner: tx,
        },
        ChannelMixedRx {
            channel_identifier: name,
            inner: rx,
        },
    )
}
