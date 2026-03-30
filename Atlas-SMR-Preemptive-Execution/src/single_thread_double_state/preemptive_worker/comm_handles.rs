use crate::single_thread_double_state::state_management::{ConfirmedToPreemptiveMsg, PreemptiveStateMessage, PreemptiveToConfirmedMsg};
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::UpdateBatch;
use getset::Getters;
use thiserror::Error;
use tracing::error;
use atlas_common::channel;


/// Messages sent by the orchestrator to the preemptive state management thread to trigger updates to the preemptive state.
pub enum PreemptiveWorkMessage<R> {
    PreemptiveUpdate(UpdateBatch<R>),
    ConfirmedUpdate(SeqNo),
    PollStateChannel
}

#[derive(Getters)]
pub(super) struct PreemptiveChannels<R, S>
{
    #[get = "pub"]
    state_rx: ChannelSyncRx<PreemptiveStateMessage<S>>,
    #[get = "pub"]
    work_rx: ChannelSyncRx<PreemptiveWorkMessage<R>>,
    #[get = "pub"]
    confirmed_worker_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<R>>,
    #[get = "pub"]
    confirmed_worker_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
}

impl<R, S> PreemptiveChannels<R, S>
{
    pub fn new(
        state_rx: ChannelSyncRx<PreemptiveStateMessage<S>>,
        work_rx: ChannelSyncRx<PreemptiveWorkMessage<R>>,
        confirmed_worker_tx: ChannelSyncTx<PreemptiveToConfirmedMsg<R>>,
        confirmed_worker_rx: ChannelSyncRx<ConfirmedToPreemptiveMsg<S>>,
    ) -> Self {
        Self {
            state_rx,
            work_rx,
            confirmed_worker_tx,
            confirmed_worker_rx,
        }
    }

    pub fn send_update_confirmed(&self, update_batch: UpdateBatch<R>) {
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

impl<R, S> Clone for PreemptiveChannels<R, S>
{
    fn clone(&self) -> Self {
        Self {
            state_rx: self.state_rx.clone(),
            work_rx: self.work_rx.clone(),
            confirmed_worker_tx: self.confirmed_worker_tx.clone(),
            confirmed_worker_rx: self.confirmed_worker_rx.clone(),
        }
    }
}

#[derive(Error, Debug)]
pub(super) enum RequestLatestStateError {
    #[error("Failed to send state copy request to confirmed worker: {0}")]
    SendRequestFailed(#[from] channel::SendError),

    #[error("Failed to receive state copy from confirmed worker: {0}")]
    ReceiveFailed(#[from] channel::RecvError),

    #[error(
        "Received unexpected message in confirmed worker while requesting state copy. This channel must be empty when requesting the state copy."
    )]
    UnexpectedMessage,
}
