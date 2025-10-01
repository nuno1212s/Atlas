use crate::channel::{
    RecvError, SendError, SendReturnError, TryRecvError, TrySendError, TrySendReturnError,
};
use crate::Err;
use crossbeam_channel::{Receiver, RecvTimeoutError, SendTimeoutError, Sender};
use std::ops::Deref;
use std::time::Duration;

pub struct ChannelSyncRx<T> {
    inner: crossbeam_channel::Receiver<T>,
}

pub struct ChannelSyncTx<T> {
    inner: crossbeam_channel::Sender<T>,
}

impl<T> Clone for ChannelSyncTx<T> {
    fn clone(&self) -> Self {
        ChannelSyncTx {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelSyncRx<T> {
    fn clone(&self) -> Self {
        ChannelSyncRx {
            inner: self.inner.clone(),
        }
    }
}

impl<T> ChannelSyncTx<T> {
    #[inline]
    pub fn send_return(&self, value: T) -> std::result::Result<(), SendReturnError<T>> {
        match self.inner.send(value) {
            Ok(_) => Ok(()),
            Err(err) => Err(SendReturnError::FailedToSend(err.into_inner())),
        }
    }

    #[inline]
    pub fn send_return_timeout(
        &self,
        value: T,
        timeout: Duration,
    ) -> std::result::Result<(), TrySendReturnError<T>> {
        match self.inner.send_timeout(value, timeout) {
            Ok(_) => Ok(()),
            Err(err) => match err {
                SendTimeoutError::Timeout(t) => Err(TrySendReturnError::Timeout(t)),
                SendTimeoutError::Disconnected(t) => Err(TrySendReturnError::Disconnected(t)),
            },
        }
    }

    #[inline]
    pub fn try_send_return(&self, value: T) -> std::result::Result<(), TrySendReturnError<T>> {
        match self.inner.try_send(value) {
            Ok(_) => Ok(()),
            Err(err) => match err {
                crossbeam_channel::TrySendError::Full(value) => {
                    Err(TrySendReturnError::Full(value))
                }
                crossbeam_channel::TrySendError::Disconnected(value) => {
                    Err(TrySendReturnError::Disconnected(value))
                }
            },
        }
    }
}

impl<T> ChannelSyncTx<T> {
    #[inline]
    pub fn send(&self, value: T) -> std::result::Result<(), SendError> {
        match self.inner.send(value) {
            Ok(_) => Ok(()),
            Err(_) => Err(SendError::FailedToSend),
        }
    }

    #[inline]
    pub fn send_timeout(
        &self,
        value: T,
        timeout: Duration,
    ) -> std::result::Result<(), TrySendError> {
        match self.inner.send_timeout(value, timeout) {
            Ok(_) => Ok(()),
            Err(err) => match err {
                SendTimeoutError::Timeout(_) => Err(TrySendError::Timeout),
                SendTimeoutError::Disconnected(_) => Err(TrySendError::Disconnected),
            },
        }
    }

    #[inline]
    pub fn try_send(&self, value: T) -> std::result::Result<(), TrySendError> {
        match self.inner.try_send(value) {
            Ok(_) => Ok(()),
            Err(err) => match err {
                crossbeam_channel::TrySendError::Full(_) => Err(TrySendError::Full),
                crossbeam_channel::TrySendError::Disconnected(_) => Err(TrySendError::Disconnected),
            },
        }
    }
}

impl<T> ChannelSyncRx<T> {
    #[inline]
    pub fn try_recv(&self) -> std::result::Result<T, TryRecvError> {
        match self.inner.try_recv() {
            Ok(res) => Ok(res),
            Err(err) => match err {
                crossbeam_channel::TryRecvError::Empty => Err(TryRecvError::ChannelEmpty),
                crossbeam_channel::TryRecvError::Disconnected => Err(TryRecvError::ChannelDc),
            },
        }
    }

    #[inline]
    pub fn recv(&self) -> Result<T, RecvError> {
        self.inner.recv().map_err(|_| RecvError::ChannelDc)
    }

    #[inline]
    pub fn recv_timeout(&self, timeout: Duration) -> std::result::Result<T, TryRecvError> {
        match self.inner.recv_timeout(timeout) {
            Ok(result) => Ok(result),
            Err(err) => match err {
                RecvTimeoutError::Timeout => {
                    Err!(TryRecvError::Timeout)
                }
                RecvTimeoutError::Disconnected => {
                    Err!(TryRecvError::ChannelDc)
                }
            },
        }
    }
}

//TODO: Maybe make this actually implement the methods so we can return our own errors?
impl<T> Deref for ChannelSyncRx<T> {
    type Target = crossbeam_channel::Receiver<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> Deref for ChannelSyncTx<T> {
    type Target = crossbeam_channel::Sender<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[inline]
pub(super) fn new_bounded<T>(bound: usize) -> (ChannelSyncTx<T>, ChannelSyncRx<T>) {
    let (tx, rx) = crossbeam_channel::bounded(bound);

    (ChannelSyncTx { inner: tx }, ChannelSyncRx { inner: rx })
}

#[inline]
pub(super) fn new_unbounded<T>() -> (ChannelSyncTx<T>, ChannelSyncRx<T>) {
    let (tx, rx) = crossbeam_channel::unbounded();

    (ChannelSyncTx { inner: tx }, ChannelSyncRx { inner: rx })
}

impl<T> From<Sender<T>> for ChannelSyncTx<T> {
    fn from(value: Sender<T>) -> Self {
        Self { inner: value }
    }
}

impl<T> From<Receiver<T>> for ChannelSyncRx<T> {
    fn from(value: Receiver<T>) -> Self {
        Self { inner: value }
    }
}

impl<T> From<ChannelSyncTx<T>> for Sender<T> {
    fn from(value: ChannelSyncTx<T>) -> Self {
        value.inner
    }
}

impl<T> From<ChannelSyncRx<T>> for Receiver<T> {
    fn from(value: ChannelSyncRx<T>) -> Self {
        value.inner
    }
}

impl<T> AsRef<Sender<T>> for ChannelSyncTx<T> {
    fn as_ref(&self) -> &Sender<T> {
        &self.inner
    }
}

impl<T> AsRef<Receiver<T>> for ChannelSyncRx<T> {
    fn as_ref(&self) -> &Receiver<T> {
        &self.inner
    }
}
