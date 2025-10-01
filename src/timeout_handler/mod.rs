use anyhow::Context;
use atlas_common::channel::sync::ChannelSyncTx;
use atlas_core::timeouts::{Timeout, TimeoutWorkerResponder};

#[derive(Clone)]
pub(crate) struct CLITimeoutHandler {
    tx: ChannelSyncTx<Vec<Timeout>>,
}

impl TimeoutWorkerResponder for CLITimeoutHandler {
    fn report_timeouts(&self, timeouts: Vec<Timeout>) -> atlas_common::error::Result<()> {
        self.tx
            .send(timeouts)
            .context("Failed to send timeouts to channel")
    }
}

impl From<ChannelSyncTx<Vec<Timeout>>> for CLITimeoutHandler {
    fn from(tx: ChannelSyncTx<Vec<Timeout>>) -> Self {
        Self { tx }
    }
}
