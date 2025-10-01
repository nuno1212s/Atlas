use crate::channel::{custom_dump, SendReturnError};

/// Channel with capability of dumping multiple members in a couple of CAS operations
pub struct ChannelMultTx<T> {
    #[cfg(feature = "channel_mult_custom_dump")]
    inner: custom_dump::ChannelTx<T>,
}

pub struct ChannelMultRx<T> {
    #[cfg(feature = "channel_mult_custom_dump")]
    inner: custom_dump::ChannelRxMult<T>,
}

impl<T> ChannelMultTx<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_dc(&self) -> bool {
        self.inner.is_dc()
    }

    #[inline]
    pub async fn send_async(&self, value: T) -> Result<(), SendReturnError<T>> {
        self.inner.send(value).await
    }

    #[inline]
    pub fn send(&self, value: T) -> Result<(), SendReturnError<T>> {
        self.inner.send_blk(value)
    }
}

impl<T> ChannelMultRx<T> {
    pub fn is_dc(&self) -> bool {
        self.inner.is_dc()
    }

    pub async fn recv_mult(&mut self) -> crate::error::Result<Vec<T>> {
        self.inner.recv().await
    }

    pub fn recv_mult_sync(&self, dest: &mut Vec<T>) -> crate::error::Result<usize> {
        self.inner.recv_sync(dest)
    }

    pub fn try_recv_mult(&self, dest: &mut Vec<T>, rq_bound: usize) -> crate::error::Result<usize> {
        self.inner.try_recv_mult(dest, rq_bound)
    }
}

impl<T> Clone for ChannelMultRx<T> {
    fn clone(&self) -> Self {
        ChannelMultRx {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelMultTx<T> {
    fn clone(&self) -> Self {
        ChannelMultTx {
            inner: self.inner.clone(),
        }
    }
}

#[inline]
pub fn new_bounded_mult<T>(bound: usize) -> (ChannelMultTx<T>, ChannelMultRx<T>) {
    let (tx, rx) = custom_dump::bounded_mult_channel(bound);

    (ChannelMultTx { inner: tx }, ChannelMultRx { inner: rx })
}
