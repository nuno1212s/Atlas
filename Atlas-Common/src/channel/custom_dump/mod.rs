#![allow(dead_code)]

use dsrust::channels::async_ch::{ReceiverFut, ReceiverMultFut};
use dsrust::channels::queue_channel::{Receiver, ReceiverMult, Sender};

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use dsrust::queues::mqueue::MQueue;

use crate::channel::{RecvError, RecvMultError, SendReturnError};
use crate::error::*;
use crate::Err;
use futures::future::FusedFuture;

#[cfg(feature = "channel_custom_dump_lfb")]
type QueueType<T> = LFBQueue<T>;

#[cfg(any(
    feature = "channel_custom_dump_mqueue",
    not(feature = "channel_custom_dump_lfb")
))]
type QueueType<T> = MQueue<T>;

pub struct ChannelTx<T> {
    inner: Sender<T, QueueType<T>>,
}

pub struct ChannelRx<T> {
    inner: Receiver<T, QueueType<T>>,
}

pub struct ChannelRxFut<'a, T> {
    inner: ReceiverFut<'a, T, QueueType<T>>,
}

pub struct ChannelRxMult<T> {
    inner: ReceiverMult<T, QueueType<T>>,
}

pub struct ChannelRxMultFut<'a, T> {
    inner: ReceiverMultFut<'a, T, QueueType<T>>,
}

impl<T> ChannelTx<T> {
    pub fn len(&self) -> usize {
        //TODO: Add this capability to DSRust
        0
    }

    pub fn is_dc(&self) -> bool {
        self.inner.is_dc()
    }

    #[inline]
    pub async fn send(&self, message: T) -> std::result::Result<(), SendReturnError<T>> {
        match self.inner.send_async(message).await {
            Ok(_) => Ok(()),
            Err(err) => Err(SendReturnError::FailedToSend(err.0)),
        }
    }

    #[inline]
    pub fn send_blk(&self, message: T) -> std::result::Result<(), SendReturnError<T>> {
        match self.inner.send(message) {
            Ok(_) => Ok(()),
            Err(err) => Err(SendReturnError::FailedToSend(err.0)),
        }
    }
}

impl<T> ChannelRx<T> {
    pub fn is_dc(&self) -> bool {
        self.inner.is_dc()
    }

    ///Async receiver with no backoff (Turns straight to event notifications)
    #[inline]
    pub fn recv(&mut self) -> ChannelRxFut<'_, T> {
        let inner = self.inner.recv_fut();

        ChannelRxFut { inner }
    }
}

impl<T> ChannelRxMult<T> {
    pub fn is_dc(&self) -> bool {
        self.inner.is_dc()
    }

    ///Async receiver with no backoff (Turns straight to event notifications)
    #[inline]
    pub fn recv(&mut self) -> ChannelRxMultFut<'_, T> {
        let inner = self.inner.recv_fut();

        ChannelRxMultFut { inner }
    }

    #[inline]
    pub fn recv_sync(&self, dest: &mut Vec<T>) -> Result<usize> {
        match self.inner.recv_mult(dest) {
            Ok(recv) => Ok(recv),
            Err(err) => match err {
                dsrust::channels::queue_channel::RecvMultError::MalformedInputVec => {
                    Err!(RecvMultError::MalformedInputVec)
                }
                dsrust::channels::queue_channel::RecvMultError::Disconnected => {
                    Err!(RecvMultError::ChannelDc)
                }
                dsrust::channels::queue_channel::RecvMultError::UnimplementedOperation => {
                    Err!(RecvMultError::Unsupported)
                }
            },
        }
    }

    #[inline]
    pub fn try_recv_mult(&self, dest: &mut Vec<T>, _bound: usize) -> Result<usize> {
        match self.inner.try_recv_mult(dest) {
            Ok(recved) => Ok(recved),
            Err(err) => match err {
                dsrust::channels::queue_channel::RecvMultError::MalformedInputVec => {
                    Err!(RecvMultError::MalformedInputVec)
                }
                dsrust::channels::queue_channel::RecvMultError::Disconnected => {
                    Err!(RecvMultError::ChannelDc)
                }
                dsrust::channels::queue_channel::RecvMultError::UnimplementedOperation => {
                    Err!(RecvMultError::Unsupported)
                }
            },
        }
    }
}

impl<T> Clone for ChannelRx<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelTx<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelRxMult<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, T> Future for ChannelRxFut<'a, T> {
    type Output = Result<T>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx).map(|opt| match opt {
            Ok(rec) => Ok(rec),
            Err(_) => {
                Err!(RecvError::ChannelDc)
            }
        })
    }
}

impl<'a, T> FusedFuture for ChannelRxFut<'a, T> {
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

///Receiver to use with the dump method
impl<'a, T> Future for ChannelRxMultFut<'a, T> {
    type Output = Result<Vec<T>>;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx).map(|opt| match opt {
            Ok(res) => Ok(res),
            Err(err) => match err {
                dsrust::channels::queue_channel::RecvMultError::MalformedInputVec => {
                    Err!(RecvMultError::MalformedInputVec)
                }
                dsrust::channels::queue_channel::RecvMultError::Disconnected => {
                    Err!(RecvMultError::ChannelDc)
                }
                dsrust::channels::queue_channel::RecvMultError::UnimplementedOperation => {
                    Err!(RecvMultError::Unsupported)
                }
            },
        })
    }
}

impl<'a, T> FusedFuture for ChannelRxMultFut<'a, T> {
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

pub fn bounded_mult_channel<T>(bound: usize) -> (ChannelTx<T>, ChannelRxMult<T>) {
    let (tx, rx) = {
        #[cfg(feature = "channel_custom_dump_lfb")]
        {
            dsrust::channels::queue_channel::bounded_lf_queue(bound)
        }

        #[cfg(any(
            feature = "channel_custom_dump_mqueue",
            not(feature = "channel_custom_dump_lfb")
        ))]
        {
            dsrust::channels::queue_channel::bounded_mutex_backoff_queue(bound)
        }
    };

    let receiver = dsrust::channels::queue_channel::make_mult_recv_from(rx);

    (ChannelTx { inner: tx }, ChannelRxMult { inner: receiver })
}
