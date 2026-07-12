use crate::channel::{RecvError, SendError};
use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::task::{Context, Poll};

/// ASYNCHRONOUS CHANNEL
#[cfg(feature = "channel_flume_mpmc")]
type InnerAsyncChannelTx<T> = super::flume_mpmc::ChannelMixedTx<T>;
#[cfg(feature = "channel_flume_mpmc")]
type InnerAsyncChannelRx<T> = super::flume_mpmc::ChannelMixedRx<T>;

/// General purpose channel's sending half.
pub struct ChannelAsyncTx<T> {
    name: Option<Arc<str>>,
    inner: InnerAsyncChannelTx<T>,
}

/// General purpose channel's receiving half.
pub struct ChannelAsyncRx<T> {
    name: Option<Arc<str>>,
    inner: InnerAsyncChannelRx<T>,
}

#[cfg(feature = "channel_flume_mpmc")]
type InnerChannelRxFut<'a, T> = super::flume_mpmc::ChannelRxFut<'a, T>;

#[cfg(feature = "channel_async_channel_mpmc")]
type InnerChannelRxFut<'a, T> = crate::channel::async_channel_mpmc::ChannelRxFut<'a, T>;

/// Future for a general purpose channel's receiving operation.
pub struct ChannelRxFut<'a, T> {
    pub(crate) channel: Option<Arc<str>>,
    pub(crate) inner: InnerChannelRxFut<'a, T>,
}

impl<'a, T> ChannelRxFut<'a, T> {
    #[inline]
    pub(crate) fn new(inner: InnerChannelRxFut<'a, T>, channel: Option<Arc<str>>) -> Self {
        Self { channel, inner }
    }
}

#[cfg(feature = "channel_flume_mpmc")]
type InnerChannelTxFut<'a, T> = super::flume_mpmc::ChannelTxFut<'a, T>;

#[cfg(feature = "channel_async_channel_mpmc")]
type InnerChannelTxFut<'a, T> = crate::channel::async_channel_mpmc::ChannelTxFut<'a, T>;

pub struct ChannelTxFut<'a, T> {
    pub(crate) channel: Option<Arc<str>>,
    pub(crate) inner: InnerChannelTxFut<'a, T>,
}

impl<'a, T> ChannelTxFut<'a, T> {
    #[inline]
    pub(crate) fn new(inner: InnerChannelTxFut<'a, T>, channel: Option<Arc<str>>) -> Self {
        Self { channel, inner }
    }
}

impl<T> Clone for ChannelAsyncTx<T> {
    #[inline]
    fn clone(&self) -> Self {
        let inner = self.inner.clone();
        Self {
            name: self.name.clone(),
            inner,
        }
    }
}

impl<T> Clone for ChannelAsyncRx<T> {
    #[inline]
    fn clone(&self) -> Self {
        let inner = self.inner.clone();
        Self {
            name: self.name.clone(),
            inner,
        }
    }
}

impl<T> ChannelAsyncTx<T> {
    //Can have length because future mpsc doesn't implement it

    //Asynchronously send message through channel
    #[inline]
    pub fn send(&mut self, message: T) -> ChannelTxFut<'_, T> {
        ChannelTxFut::new(self.inner.send(message).into(), self.name.clone())
    }
}

impl<T> ChannelAsyncRx<T> {
    //Asynchronously recv message from channel
    #[inline]
    pub fn recv(&mut self) -> ChannelRxFut<'_, T> {
        ChannelRxFut::new(self.inner.recv().into(), self.name.clone())
    }
}

impl<'a, T> Future for ChannelRxFut<'a, T> {
    type Output = Result<T, RecvError>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<T, RecvError>> {
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

/// Creates a new general purpose channel that can queue up to
/// `bound` messages from different async senders.
#[inline]
pub fn new_bounded_async<T>(
    bound: usize,
    name: Option<impl Into<String>>,
) -> (ChannelAsyncTx<T>, ChannelAsyncRx<T>) {
    let name = name.map(|string| Arc::from(string.into()));

    let (tx, rx) = {
        #[cfg(feature = "channel_flume_mpmc")]
        {
            super::flume_mpmc::new_bounded(bound)
        }
        #[cfg(feature = "channel_async_channel_mpmc")]
        {
            super::async_channel_mpmc::new_bounded(bound)
        }
    };

    let ttx = ChannelAsyncTx {
        name: name.clone(),
        inner: tx,
    };

    let rrx = ChannelAsyncRx { name, inner: rx };

    (ttx, rrx)
}
