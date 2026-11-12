use crate::Err;
use crate::channel::{
    RecvError, SendError, SendReturnError, TryRecvError, TrySendError, TrySendReturnError,
};
use crossbeam_channel::{RecvError as CBRecvError, RecvTimeoutError, SendTimeoutError};
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
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `None` for an unbounded channel.
    #[inline]
    pub fn capacity(&self) -> Option<usize> {
        self.inner.capacity()
    }

    #[inline]
    pub fn send_return(&self, value: T) -> std::result::Result<(), SendReturnError<T>> {
        match self.inner.send(value) {
            Ok(_) => Ok(()),
            Err(err) => Err(SendReturnError::FailedToSend(err.into_inner(), None)),
        }
    }

    #[inline]
    pub fn try_send_return(&self, value: T) -> Result<(), TrySendReturnError<T>> {
        match self.inner.try_send(value) {
            Ok(_) => Ok(()),
            Err(err) => match err {
                crossbeam_channel::TrySendError::Full(value) => {
                    Err(TrySendReturnError::Full(value, None))
                }
                crossbeam_channel::TrySendError::Disconnected(value) => {
                    Err(TrySendReturnError::Disconnected(value, None))
                }
            },
        }
    }
}

impl<T> ChannelSyncTx<T> {
    #[inline]
    pub fn send(&self, value: T) -> Result<(), SendError> {
        match self.inner.send(value) {
            Ok(_) => Ok(()),
            Err(_) => Err(SendError::FailedToSend { channel: None }),
        }
    }

    #[inline]
    pub fn send_timeout(&self, value: T, timeout: Duration) -> Result<(), TrySendError> {
        match self.inner.send_timeout(value, timeout) {
            Ok(_) => Ok(()),
            Err(err) => match err {
                SendTimeoutError::Timeout(_) => Err(TrySendError::Timeout { channel: None }),
                SendTimeoutError::Disconnected(_) => {
                    Err(TrySendError::Disconnected { channel: None })
                }
            },
        }
    }

    #[inline]
    pub fn try_send(&self, value: T) -> Result<(), TrySendError> {
        match self.inner.try_send(value) {
            Ok(_) => Ok(()),
            Err(err) => match err {
                crossbeam_channel::TrySendError::Full(_) => {
                    Err(TrySendError::Full { channel: None })
                }
                crossbeam_channel::TrySendError::Disconnected(_) => {
                    Err(TrySendError::Disconnected { channel: None })
                }
            },
        }
    }
}

impl<T> ChannelSyncRx<T> {
    /// The underlying handle, for `channel::sync::__select` only. Deliberately
    /// crate-internal: nothing outside `atlas-common` should ever name the
    /// backend's types.
    #[inline]
    pub(in crate::channel) fn raw(&self) -> &crossbeam_channel::Receiver<T> {
        &self.inner
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.inner.try_recv() {
            Ok(res) => Ok(res),
            Err(err) => match err {
                crossbeam_channel::TryRecvError::Empty => {
                    Err(TryRecvError::ChannelEmpty { channel: None })
                }
                crossbeam_channel::TryRecvError::Disconnected => {
                    Err(TryRecvError::ChannelDc { channel: None })
                }
            },
        }
    }

    #[inline]
    pub fn recv(&self) -> Result<T, RecvError> {
        self.inner
            .recv()
            .map_err(|_| RecvError::ChannelDc { channel: None })
    }

    #[inline]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, TryRecvError> {
        match self.inner.recv_timeout(timeout) {
            Ok(result) => Ok(result),
            Err(err) => match err {
                RecvTimeoutError::Timeout => {
                    Err!(TryRecvError::Timeout { channel: None })
                }
                RecvTimeoutError::Disconnected => {
                    Err!(TryRecvError::ChannelDc { channel: None })
                }
            },
        }
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

impl From<CBRecvError> for RecvError {
    fn from(_: CBRecvError) -> Self {
        Self::ChannelDc { channel: None }
    }
}
