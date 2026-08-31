//! Async MPMC channel backed by `async-channel`.
//!
//! `async-channel`'s `Receiver`/`Sender` and their `Recv`/`Send` futures are all
//! deliberately `!Unpin`, so both wrappers hold the backend future and project
//! into it rather than re-pinning a `&mut` each poll.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::channel::{RecvError, SendReturnError};
use async_channel::{Receiver, Sender};

pub struct ChannelAsyncTx<T> {
    inner: Sender<T>,
}

pub struct ChannelAsyncRx<T> {
    inner: Receiver<T>,
}

pin_project_lite::pin_project! {
    pub struct ChannelRxFut<'a, T> {
        #[pin]
        inner: async_channel::Recv<'a, T>,
    }
}

pin_project_lite::pin_project! {
    pub struct ChannelTxFut<'a, T> {
        #[pin]
        inner: async_channel::Send<'a, T>,
    }
}

impl<T> Clone for ChannelAsyncTx<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelAsyncRx<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> ChannelAsyncTx<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_dc(&self) -> bool {
        self.inner.is_closed()
    }

    #[inline]
    pub fn send(&mut self, message: T) -> ChannelTxFut<'_, T> {
        ChannelTxFut {
            inner: self.inner.send(message),
        }
    }
}

impl<T> ChannelAsyncRx<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_dc(&self) -> bool {
        self.inner.is_closed()
    }

    #[inline]
    pub fn recv(&mut self) -> ChannelRxFut<'_, T> {
        ChannelRxFut {
            inner: self.inner.recv(),
        }
    }
}

impl<'a, T> Future for ChannelRxFut<'a, T> {
    type Output = Result<T, RecvError>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        self.project()
            .inner
            .poll(cx)
            .map(|res| res.map_err(|_| RecvError::ChannelDc { channel: None }))
    }
}

impl<'a, T> Future for ChannelTxFut<'a, T> {
    type Output = Result<(), SendReturnError<T>>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|r| match r {
            Ok(()) => Ok(()),
            Err(async_channel::SendError(value)) => Err(SendReturnError::FailedToSend(value, None)),
        })
    }
}

pub fn new_bounded<T>(bound: usize) -> (ChannelAsyncTx<T>, ChannelAsyncRx<T>) {
    let (tx, rx) = async_channel::bounded(bound);

    (ChannelAsyncTx { inner: tx }, ChannelAsyncRx { inner: rx })
}

pub fn new_unbounded<T>() -> (ChannelAsyncTx<T>, ChannelAsyncRx<T>) {
    let (tx, rx) = async_channel::unbounded();

    (ChannelAsyncTx { inner: tx }, ChannelAsyncRx { inner: rx })
}
