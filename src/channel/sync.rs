use crate::channel::{RecvError, SendError, SendReturnError, TrySendError};
use crate::channel::{TryRecvError, TrySendReturnError};
use std::sync::Arc;
use std::time::Duration;

/**
Sync channels
 */
#[cfg(feature = "channel_sync_crossbeam")]
type InnerSyncChannelRx<T> = super::crossbeam::ChannelSyncRx<T>;

#[cfg(feature = "channel_sync_crossbeam")]
type InnerSyncChannelTx<T> = super::crossbeam::ChannelSyncTx<T>;

#[cfg(feature = "channel_sync_flume")]
type InnerSyncChannelRx<T> = flume_mpmc::ChannelMixedRx<T>;

#[cfg(feature = "channel_sync_flume")]
type InnerSyncChannelTx<T> = flume_mpmc::ChannelMixedTx<T>;

pub struct ChannelSyncRx<T> {
    name: Option<Arc<str>>,
    inner: InnerSyncChannelRx<T>,
}

pub struct ChannelSyncTx<T> {
    channel_identifier: Option<Arc<str>>,
    inner: InnerSyncChannelTx<T>,
}

impl<T> ChannelSyncRx<T> {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.inner.try_recv()
    }

    #[inline]
    pub fn recv(&self) -> Result<T, RecvError> {
        self.inner.recv()
    }

    #[inline]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, TryRecvError> {
        self.inner.recv_timeout(timeout)
    }
}

impl<T> ChannelSyncTx<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn send(&self, value: T) -> Result<(), SendError> {
        self.inner.send(value)
    }

    #[inline]
    pub fn send_timeout(&self, value: T, timeout: Duration) -> Result<(), TrySendError> {
        self.inner.send_timeout(value, timeout)
    }

    #[inline]
    pub fn try_send(&self, value: T) -> Result<(), TrySendError> {
        self.inner.try_send(value)
    }
}

impl<T> ChannelSyncTx<T> {
    #[inline]
    pub fn send_return(&self, value: T) -> Result<(), SendReturnError<T>> {
        let value = match self.inner.try_send_return(value) {
            Ok(_) => {
                return Ok(());
            }
            Err(err) => match err {
                TrySendReturnError::Full(value) => {
                    tracing::error!(
                        channel = self.channel_identifier.as_deref().unwrap_or("Unknown"),
                        capacity = self.inner.capacity(),
                        current_occupation = self.inner.len(),
                        "Failed to insert into channel. Channel is full and could not directly insert, blocking",);

                    value
                }
                TrySendReturnError::Disconnected(value) => {
                    tracing::error!("Channel is disconnected");

                    value
                }
                TrySendReturnError::Timeout(value) => value,
            },
        };

        self.inner.send_return(value)
    }

    #[inline]
    pub fn try_send_return(&self, value: T) -> Result<(), TrySendReturnError<T>> {
        self.inner.try_send_return(value)
    }
}

impl<T> Clone for ChannelSyncTx<T> {
    fn clone(&self) -> Self {
        ChannelSyncTx {
            channel_identifier: self.channel_identifier.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelSyncRx<T> {
    fn clone(&self) -> Self {
        ChannelSyncRx {
            name: self.name.clone(),
            inner: self.inner.clone(),
        }
    }
}

#[inline]
pub fn new_bounded_sync<T>(
    bound: usize,
    name: Option<impl Into<String>>,
) -> (ChannelSyncTx<T>, ChannelSyncRx<T>) {
    let name = name.map(|string| Arc::from(string.into()));

    let (tx, rx) = {
        #[cfg(feature = "channel_sync_crossbeam")]
        {
            super::crossbeam::new_bounded(bound)
        }
        #[cfg(feature = "channel_sync_flume")]
        {
            super::flume_mpmc::new_bounded(bound)
        }
    };

    (
        ChannelSyncTx {
            channel_identifier: name.clone(),
            inner: tx,
        },
        ChannelSyncRx { name, inner: rx },
    )
}

#[inline]
pub fn new_unbounded_sync<T>(
    name: Option<impl Into<String>>,
) -> (ChannelSyncTx<T>, ChannelSyncRx<T>) {
    let name = name.map(|string| Arc::from(string.into()));

    let (tx, rx) = {
        #[cfg(feature = "channel_sync_crossbeam")]
        {
            super::crossbeam::new_unbounded()
        }
        #[cfg(feature = "channel_sync_flume")]
        {
            super::flume_mpmc::new_unbounded()
        }
    };

    (
        ChannelSyncTx {
            channel_identifier: name.clone(),
            inner: tx,
        },
        ChannelSyncRx { name, inner: rx },
    )
}

impl<T> From<ChannelSyncTx<T>> for InnerSyncChannelTx<T> {
    fn from(value: ChannelSyncTx<T>) -> Self {
        value.inner
    }
}

impl<T> From<ChannelSyncRx<T>> for InnerSyncChannelRx<T> {
    fn from(value: ChannelSyncRx<T>) -> Self {
        value.inner
    }
}

impl<T> AsRef<InnerSyncChannelTx<T>> for ChannelSyncTx<T> {
    fn as_ref(&self) -> &InnerSyncChannelTx<T> {
        &self.inner
    }
}

impl<T> AsRef<InnerSyncChannelRx<T>> for ChannelSyncRx<T> {
    fn as_ref(&self) -> &InnerSyncChannelRx<T> {
        &self.inner
    }
}

#[macro_export]
macro_rules! unwrap_channel {
    ($channel: expr) => {
        $channel.as_ref().as_ref()
    };
}

#[macro_export]
macro_rules! exhaust_and_consume {
    ($existing_msg: expr, $channel: expr, $self_obj: expr, $consumption: ident) => {{
        $self_obj.$consumption($existing_msg)?;

        while let Ok(message) = $channel.try_recv() {
            $self_obj.$consumption(message)?;
        }

        Ok(())
    }};
    ($channel:expr, $self_obj:expr, $consumption:ident) => {
        while let Ok(message) = $channel.try_recv() {
            $self_obj.$consumption(message)?;
        }
    };
    ($channel: expr, $self_obj:expr, $consumption:ident, $timeout: expr) => {
        while let Ok(message) = $channel.recv_timeout($timeout) {
            $self_obj.$consumption(message)?;
        }
    };
}

#[cfg(feature = "channel_sync_crossbeam")]
extern crate crossbeam_channel;

/// To use the sync_select macro,
/// you will have to combine it with the unwrap channel macro found above
#[cfg(feature = "channel_sync_crossbeam")]
pub use crossbeam_channel::{select as sync_select, select as sync_select_biased};
