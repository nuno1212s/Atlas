use crate::single_thread_double_state::state_management::{
    ConfirmedToPreemptiveMsg, PreemptiveStateMessage, PreemptiveToConfirmedMsg,
};
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::UpdateBatch;
use atlas_smr_application::app::{Application, Request};
use thiserror::Error;
use tracing::error;

pub(super) struct PreemptiveChannels<A, S>
where
    A: Application<S>,
{
    work_rx: ChannelSyncRx<PreemptiveStateMessage<A, S>>,
    confirmed_worker_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<A, S>>,
    confirmed_worker_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
}

impl<A, S> PreemptiveChannels<A, S>
where
    A: Application<S>,
{
    pub fn new(
        work_rx: ChannelSyncRx<PreemptiveStateMessage<A, S>>,
        confirmed_worker_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<A, S>>,
        confirmed_worker_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
    ) -> Self {
        Self {
            work_rx,
            confirmed_worker_tx,
            confirmed_worker_rx,
        }
    }

    pub fn send_update_confirmed(&self, update_batch: UpdateBatch<Request<A, S>>) {
        if let Err(err) = self
            .confirmed_worker_tx
            .send(PreemptiveToConfirmedMsg::UpdateConfirmed(update_batch))
        {
            error!("Failed to send update batch to confirmed worker: {err}");
        }
    }

    pub fn request_latest_confirmed_state(
        &self,
        seq_no: SeqNo,
    ) -> Result<(SeqNo, S), RequestLatestStateError> {
        if !self.confirmed_worker_rx.is_empty() {
            return Err(RequestLatestStateError::UnexpectedMessage);
        }

        self.confirmed_worker_tx
            .send(PreemptiveToConfirmedMsg::RequestStateCopy(seq_no))?;

        match self.confirmed_worker_rx.recv() {
            Ok(ConfirmedToPreemptiveMsg(confirmed_seq_no, confirmed_state)) => {
                Ok((confirmed_seq_no, confirmed_state))
            }
            Err(err) => Err(RequestLatestStateError::ReceiveFailed(err)),
        }
    }
}

#[derive(Error, Debug)]
pub(super) enum RequestLatestStateError {
    #[error("Failed to send state copy request to confirmed worker: {0}")]
    SendRequestFailed(#[from] atlas_common::channel::SendError),

    #[error("Failed to receive state copy from confirmed worker: {0}")]
    ReceiveFailed(#[from] atlas_common::channel::RecvError),

    #[error(
        "Received unexpected message in confirmed worker while requesting state copy. This channel must be empty when requesting the state copy."
    )]
    UnexpectedMessage,
}
