#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;

use crate::channel::{RecvError, SendReturnError, TryRecvError, TrySendReturnError};
use flume::r#async::{RecvFut, SendFut};
use flume::{RecvTimeoutError, SendTimeoutError};
use futures::future::FusedFuture;
use std::task::{Context, Poll};
use std::time::Duration;

/**
Mixed channels
 */
pub struct ChannelMixedRx<T> {
    inner: flume::Receiver<T>,
}

pub struct ChannelMixedTx<T> {
    inner: ::flume::Sender<T>,
}

pub struct ChannelRxFut<'a, T> {
    inner: RecvFut<'a, T>,
}

pub struct ChannelTxFut<'a, T> {
    inner: SendFut<'a, T>,
}

impl<T> Clone for ChannelMixedTx<T> {
    fn clone(&self) -> Self {
        ChannelMixedTx {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelMixedRx<T> {
    fn clone(&self) -> Self {
        ChannelMixedRx {
            inner: self.inner.clone(),
        }
    }
}

impl<T> ChannelMixedTx<T> {
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_dc(&self) -> bool {
        self.inner.is_disconnected()
    }

    #[inline]
    pub fn send(&self, message: T) -> ChannelTxFut<'_, T> {
        self.inner.send_async(message).into()
    }

    #[inline]
    pub fn send_sync(&self, message: T) -> Result<(), SendReturnError<T>> {
        match self.inner.send(message) {
            Ok(_) => Ok(()),
            Err(err) => Err(SendReturnError::FailedToSend(err.into_inner())),
        }
    }

    #[inline]
    pub fn send_sync_return(&self, message: T) -> Result<(), SendReturnError<T>> {
        self.send_sync(message)
    }

    #[inline]
    pub fn send_timeout_sync(
        &self,
        message: T,
        timeout: Duration,
    ) -> Result<(), TrySendReturnError<T>> {
        match self.inner.send_timeout(message, timeout) {
            Ok(_) => Ok(()),
            Err(err) => match err {
                SendTimeoutError::Timeout(e) => Err(TrySendReturnError::Timeout(e)),
                SendTimeoutError::Disconnected(e) => Err(TrySendReturnError::Disconnected(e)),
            },
        }
    }

    #[inline]
    pub fn send_timeout_sync_return(
        &self,
        message: T,
        timeout: Duration,
    ) -> Result<(), TrySendReturnError<T>> {
        self.send_timeout_sync(message, timeout)
    }
}

impl<T> ChannelMixedRx<T> {
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_dc(&self) -> bool {
        self.inner.is_disconnected()
    }

    #[inline]
    pub fn recv(&mut self) -> ChannelRxFut<'_, T> {
        self.inner.recv_async().into()
    }
}

impl<T> ChannelMixedRx<T> {
    #[inline]
    pub fn recv_sync(&self) -> Result<T, RecvError> {
        match self.inner.recv() {
            Ok(elem) => Ok(elem),
            Err(_) => Err(RecvError::ChannelDc),
        }
    }

    #[inline]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, TryRecvError> {
        match self.inner.recv_timeout(timeout) {
            Ok(elem) => Ok(elem),
            Err(err) => match err {
                RecvTimeoutError::Timeout => Err(TryRecvError::Timeout),
                RecvTimeoutError::Disconnected => Err(TryRecvError::ChannelDc),
            },
        }
    }

    #[inline]
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.inner.try_recv() {
            Ok(ele) => Ok(ele),
            Err(err) => match err {
                flume::TryRecvError::Empty => Err(TryRecvError::ChannelEmpty),
                flume::TryRecvError::Disconnected => Err(TryRecvError::ChannelDc),
            },
        }
    }
}

impl<'a, T> Future for ChannelRxFut<'a, T> {
    type Output = Result<T, RecvError>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<T, RecvError>> {
        Pin::new(&mut self.inner).poll(cx).map(|r| match r {
            Ok(res) => Ok(res),
            Err(_) => Err(RecvError::ChannelDc),
        })
    }
}

impl<'a, T> FusedFuture for ChannelRxFut<'a, T> {
    #[inline]
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

impl<'a, T> Future for ChannelTxFut<'a, T> {
    type Output = Result<(), SendReturnError<T>>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), SendReturnError<T>>> {
        Pin::new(&mut self.inner).poll(cx).map(|r| match r {
            Ok(_) => Ok(()),
            Err(err) => Err(SendReturnError::FailedToSend(err.into_inner())),
        })
    }
}

impl<'a, T> FusedFuture for ChannelTxFut<'a, T> {
    #[inline]
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

impl<'a, T> From<SendFut<'a, T>> for ChannelTxFut<'a, T> {
    fn from(value: SendFut<'a, T>) -> Self {
        Self { inner: value }
    }
}

impl<'a, T> From<RecvFut<'a, T>> for ChannelRxFut<'a, T> {
    fn from(value: RecvFut<'a, T>) -> Self {
        Self { inner: value }
    }
}

impl<'a, T> From<ChannelTxFut<'a, T>> for super::r#async::ChannelTxFut<'a, T> {
    fn from(value: ChannelTxFut<'a, T>) -> Self {
        Self { inner: value }
    }
}

impl<'a, T> From<ChannelRxFut<'a, T>> for super::r#async::ChannelRxFut<'a, T> {
    fn from(value: ChannelRxFut<'a, T>) -> Self {
        Self { inner: value }
    }
}

pub fn new_bounded<T>(bound: usize) -> (ChannelMixedTx<T>, ChannelMixedRx<T>) {
    let (tx, rx) = flume::bounded(bound);

    (ChannelMixedTx { inner: tx }, ChannelMixedRx { inner: rx })
}

/// Generate a new unbounded channel from flume, wrap it in our own channel and return it
pub fn new_unbounded<T>() -> (ChannelMixedTx<T>, ChannelMixedRx<T>) {
    let (tx, rx) = flume::unbounded();

    (ChannelMixedTx { inner: tx }, ChannelMixedRx { inner: rx })
}
