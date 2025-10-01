use crate::channel::{RecvError, SendReturnError, TryRecvError, TrySendReturnError};
use std::sync::Arc;
use std::time::Duration;

/**
Async and sync mixed channels (Allows us to connect async and sync environments together)
 */
use super::r#async::{ChannelRxFut, ChannelTxFut};

#[cfg(feature = "channel_mixed_flume")]
type InnerChannelMixedRx<T> = super::flume_mpmc::ChannelMixedRx<T>;

#[cfg(feature = "channel_mixed_flume")]
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
        match self.inner.recv_sync() {
            Ok(res) => Ok(res),
            Err(_err) => Err(RecvError::ChannelDc),
        }
    }

    #[inline]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, TryRecvError> {
        self.inner.recv_timeout(timeout)
    }

    #[inline]
    pub fn recv_async(&mut self) -> ChannelRxFut<'_, T> {
        self.inner.recv().into()
    }

    #[inline]
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.inner.try_recv()
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
        self.inner.send(value).into()
    }

    #[inline]
    pub fn send_async_return(&self, value: T) -> ChannelTxFut<'_, T> {
        self.inner.send(value).into()
    }

    #[inline]
    pub fn send(&self, value: T) -> crate::error::Result<()> {
        Ok(self.inner.send_sync(value)?)
    }

    #[inline]
    pub fn send_return(&self, value: T) -> Result<(), SendReturnError<T>> {
        self.inner.send_sync_return(value)
    }

    pub fn send_timeout(&self, value: T, timeout: Duration) -> crate::error::Result<()> {
        Ok(self.inner.send_timeout_sync(value, timeout)?)
    }

    #[inline]
    pub fn send_timeout_return(
        &self,
        value: T,
        timeout: Duration,
    ) -> Result<(), TrySendReturnError<T>> {
        self.inner.send_timeout_sync_return(value, timeout)
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

    let (tx, rx) = {
        #[cfg(feature = "channel_mixed_flume")]
        {
            super::flume_mpmc::new_bounded(bound)
        }
    };

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
