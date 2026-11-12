//! FIFO channels used to send messages between async tasks.

use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use thiserror::Error;

/// Rendered when an error is not associated with a named channel.
const UNNAMED_CHANNEL: &str = "unidentified";

mod flume_mpmc;

#[cfg(feature = "channel_async_channel_mpmc")]
mod async_channel_mpmc;

mod custom_dump;

#[cfg(not(feature = "channel_sync_flume"))]
mod crossbeam;

#[cfg(feature = "channel_sync_flume")]
mod flume_sync;

mod oneshot_spsc;

pub mod r#async;
pub mod mixed;
pub mod mult;
pub mod oneshot;
pub mod sync;

#[derive(Error, Debug)]
pub enum NoRetChannelErr {
    #[error("{0}")]
    RecvMult(#[from] RecvMultError),
    #[error("{0}")]
    Recv(#[from] RecvError),
    #[error("{0}")]
    TryRecv(#[from] TryRecvError),
    #[error("{0}")]
    Send(#[from] SendError),
    #[error("{0}")]
    TrySend(#[from] TrySendError),
}

/**
Errors
 **/
#[derive(Error, Debug)]
pub enum RecvMultError {
    #[error("Failed receive, channel is disconnected")]
    ChannelDc,
    #[error("The input vec to place received messages is malformed")]
    MalformedInputVec,
    #[error("Unsupported operation")]
    Unsupported,
}

#[derive(Error, Debug)]
pub enum TryRecvError {
    #[error("[Channel: {}] Channel has disconnected", .channel.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    ChannelDc { channel: Option<Arc<str>> },
    #[error("[Channel: {}] Channel is empty", .channel.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    ChannelEmpty { channel: Option<Arc<str>> },
    #[error("[Channel: {}] Receive operation timed out", .channel.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    Timeout { channel: Option<Arc<str>> },
}

impl TryRecvError {
    /// Attach the identifier of the channel that produced this error.
    pub fn with_channel(self, channel: Option<Arc<str>>) -> Self {
        match self {
            TryRecvError::ChannelDc { .. } => TryRecvError::ChannelDc { channel },
            TryRecvError::ChannelEmpty { .. } => TryRecvError::ChannelEmpty { channel },
            TryRecvError::Timeout { .. } => TryRecvError::Timeout { channel },
        }
    }
}

#[derive(Error, Debug)]
pub enum RecvError {
    #[error("[Channel: {}] Channel has disconnected", .channel.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    ChannelDc { channel: Option<Arc<str>> },
}

impl RecvError {
    /// Attach the identifier of the channel that produced this error.
    pub fn with_channel(self, channel: Option<Arc<str>>) -> Self {
        match self {
            RecvError::ChannelDc { .. } => RecvError::ChannelDc { channel },
        }
    }
}

#[derive(Error)]
pub enum TrySendReturnError<T> {
    #[error("[Channel: {}] Channel has disconnected", .1.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    Disconnected(T, Option<Arc<str>>),
    #[error("[Channel: {}] Send operation has timed out", .1.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    Timeout(T, Option<Arc<str>>),
    #[error("[Channel: {}] Channel is full", .1.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    Full(T, Option<Arc<str>>),
}

impl<T> TrySendReturnError<T> {
    /// Attach the identifier of the channel that produced this error.
    pub fn with_channel(self, channel: Option<Arc<str>>) -> Self {
        match self {
            TrySendReturnError::Disconnected(value, _) => {
                TrySendReturnError::Disconnected(value, channel)
            }
            TrySendReturnError::Timeout(value, _) => TrySendReturnError::Timeout(value, channel),
            TrySendReturnError::Full(value, _) => TrySendReturnError::Full(value, channel),
        }
    }
}

impl<T> From<TrySendReturnError<T>> for TrySendError {
    fn from(value: TrySendReturnError<T>) -> Self {
        match value {
            TrySendReturnError::Disconnected(_, channel) => TrySendError::Disconnected { channel },
            TrySendReturnError::Timeout(_, channel) => TrySendError::Timeout { channel },
            TrySendReturnError::Full(_, channel) => TrySendError::Full { channel },
        }
    }
}

#[derive(Error, Debug)]
pub enum SendError {
    #[error("[Channel: {}] Failed to send message", .channel.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    FailedToSend { channel: Option<Arc<str>> },
}

impl SendError {
    /// Attach the identifier of the channel that produced this error.
    pub fn with_channel(self, channel: Option<Arc<str>>) -> Self {
        match self {
            SendError::FailedToSend { .. } => SendError::FailedToSend { channel },
        }
    }
}

#[derive(Error, Debug)]
pub enum TrySendError {
    #[error("[Channel: {}] Channel has disconnected", .channel.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    Disconnected { channel: Option<Arc<str>> },
    #[error("[Channel: {}] Send operation has timed out", .channel.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    Timeout { channel: Option<Arc<str>> },
    #[error("[Channel: {}] Channel is full", .channel.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    Full { channel: Option<Arc<str>> },
}

impl TrySendError {
    /// Attach the identifier of the channel that produced this error.
    pub fn with_channel(self, channel: Option<Arc<str>>) -> Self {
        match self {
            TrySendError::Disconnected { .. } => TrySendError::Disconnected { channel },
            TrySendError::Timeout { .. } => TrySendError::Timeout { channel },
            TrySendError::Full { .. } => TrySendError::Full { channel },
        }
    }
}

#[derive(Error)]
pub enum SendReturnError<T> {
    #[error("[Channel: {}] Failed to send message, channel disconnected", .1.as_deref().unwrap_or(UNNAMED_CHANNEL))]
    FailedToSend(T, Option<Arc<str>>),
}

impl<T> SendReturnError<T> {
    /// Attach the identifier of the channel that produced this error.
    pub fn with_channel(self, channel: Option<Arc<str>>) -> Self {
        match self {
            SendReturnError::FailedToSend(value, _) => {
                SendReturnError::FailedToSend(value, channel)
            }
        }
    }
}

impl<T> From<SendReturnError<T>> for SendError {
    fn from(value: SendReturnError<T>) -> Self {
        match value {
            SendReturnError::FailedToSend(_, channel) => SendError::FailedToSend { channel },
        }
    }
}

unsafe impl<T> Send for SendReturnError<T> {}

unsafe impl<T> Sync for SendReturnError<T> {}

unsafe impl<T> Send for TrySendReturnError<T> {}

unsafe impl<T> Sync for TrySendReturnError<T> {}

impl<T> Debug for SendReturnError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SendReturnError::FailedToSend(_, channel) => write!(
                f,
                "[Channel: {}] Failed to send message",
                channel.as_deref().unwrap_or(UNNAMED_CHANNEL)
            ),
        }
    }
}

impl<T> Debug for TrySendReturnError<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let (reason, channel) = match self {
            TrySendReturnError::Disconnected(_, channel) => ("channel disconnected", channel),
            TrySendReturnError::Timeout(_, channel) => ("send timed out", channel),
            TrySendReturnError::Full(_, channel) => ("channel full", channel),
        };

        write!(
            f,
            "[Channel: {}] Failed to send message ({reason})",
            channel.as_deref().unwrap_or(UNNAMED_CHANNEL)
        )
    }
}
