use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::channel::{RecvError, SendError, SendReturnError};
use crate::error::*;
use crate::Err;
use async_channel::{Receiver, Recv, Sender};
use futures::future::FusedFuture;
use futures::stream::{FusedStream, Stream};

pub struct ChannelAsyncTx<T> {
    inner: Sender<T>,
}

pub struct ChannelAsyncRx<T> {
    inner: Receiver<T>,
}

pub struct ChannelRxFut<'a, T> {
    inner: async_channel::Recv<'a, T>,
}

pub struct ChannelTxFut<'a, T> {
    inner: async_channel::Send<'a, T>,
}

impl<T> Clone for ChannelAsyncTx<T> {
    fn clone(&self) -> Self {
        let inner = self.inner.clone();
        Self { inner }
    }
}

impl<T> Clone for ChannelAsyncRx<T> {
    fn clone(&self) -> Self {
        let inner = self.inner.clone();
        Self { inner }
    }
}

pub fn new_bounded<T>(bound: usize) -> (ChannelAsyncTx<T>, ChannelAsyncRx<T>) {
    let (tx, rx) = async_channel::bounded(bound);
    let tx = ChannelAsyncTx { inner: tx };
    let rx = ChannelAsyncRx { inner: rx };
    (tx, rx)
}

impl<T> ChannelAsyncTx<T> {
    #[inline]
    pub fn send(&mut self, message: T) -> ChannelTxFut<'_, T> {
        self.inner.send(message).into()
    }
}

impl<T> ChannelAsyncRx<T> {
    #[inline]
    pub fn recv<'a>(&'a mut self) -> ChannelRxFut<'a, T> {
        self.inner.recv().into()
    }
}

impl<'a, T> Future for ChannelRxFut<'a, T> {
    type Output = Result<T>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<T>> {
        Pin::new(&mut self.inner)
            .poll_next(cx)
            .map(|opt| opt.ok_or(RecvError::ChannelDc.into()))
    }
}

impl<'a, T> FusedFuture for ChannelRxFut<'a, T> {
    #[inline]
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

impl<'a, T> Future for ChannelTxFut<'a, T> {
    type Output = Result<()>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<()>> {
        Pin::new(&mut self.inner)
            .poll_next(cx)
            .map(|opt| opt.ok_or(SendError::ChannelDc.into()))
    }
}

impl<'a, T> FusedFuture for ChannelTxFut<'a, T> {
    #[inline]
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

impl<'a, T> From<Recv<'a, T>> for ChannelRxFut<'a, T> {
    fn from(inner: Recv<'a, T>) -> Self {
        Self { inner }
    }
}

impl<'a, T> From<async_channel::Send<'a, T>> for ChannelTxFut<'a, T> {
    fn from(inner: async_channel::Send<'a, T>) -> Self {
        Self { inner }
    }
}
