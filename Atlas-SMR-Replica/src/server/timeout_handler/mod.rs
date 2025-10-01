use atlas_common::channel::sync::ChannelSyncTx;
use atlas_core::timeouts::{Timeout, TimeoutWorkerResponder};

#[derive(Clone)]
pub(crate) struct TimeoutHandler {
    tx: ChannelSyncTx<Vec<Timeout>>,
}

impl TimeoutWorkerResponder for TimeoutHandler {
    fn report_timeouts(&self, timeouts: Vec<Timeout>) -> atlas_common::error::Result<()> {
        self.tx.send(timeouts)
    }
}

impl From<ChannelSyncTx<Vec<Timeout>>> for TimeoutHandler {
    fn from(value: ChannelSyncTx<Vec<Timeout>>) -> Self {
        Self { tx: value }
    }
}
