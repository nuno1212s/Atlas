use std::marker::PhantomData;
use std::sync::Arc;

use crate::server::IterableProtocolRes;
use atlas_common::channel;
use atlas_common::channel::oneshot::OneShotTx;
use atlas_common::channel::sync::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::ordering::SeqNo;
use atlas_communication::message::StoredMessage;
use atlas_communication::stub::RegularNetworkStub;
use atlas_core::ordering_protocol::networking::serialize::NetworkView;
use atlas_core::ordering_protocol::ExecutionResult;
use atlas_core::timeouts::timeout::ModTimeout;
use atlas_smr_core::serialize::StateSys;
use atlas_smr_core::state_transfer::networking::StateTransferSendNode;
use atlas_smr_core::state_transfer::{CstM, STPollResult, STResult, StateTransferProtocol};
use either::Either;
use getset::Getters;

pub const WORK_CHANNEL_SIZE: usize = 128;
pub const RESPONSE_CHANNEL_SIZE: usize = 128;

/// A state transfer work message
pub enum StateTransferWorkMessage<V>
where
    V: NetworkView,
{
    RequestLatestState(V),
    ViewState(V),
    Timeout(V, Vec<ModTimeout>),
    ShouldRequestAppState(SeqNo, OneShotTx<ExecutionResult>),
}

/// Response message of the state transfer protocol
pub enum StateTransferProgress {
    StateTransferProgress(STResult),
    CheckpointReceived(SeqNo),
}

/// The handle to the state transfer thread.
#[derive(Getters)]
pub struct StateTransferThreadHandle<V>
where
    V: NetworkView,
{
    work_tx: ChannelSyncTx<StateTransferWorkMessage<V>>,
    #[get = "pub"]
    response_rx: ChannelSyncRx<StateTransferProgress>,
}

#[derive(Clone, Getters)]
pub struct StateTransferThreadInnerHandle<V>
where
    V: NetworkView,
{
    #[get = "pub"]
    work_rx: ChannelSyncRx<StateTransferWorkMessage<V>>,
    #[get = "pub"]
    response_tx: ChannelSyncTx<StateTransferProgress>,
}

/// The state transfer management struct, contains the base work handles to deliver work to the
/// state transfer module
#[derive(Getters)]
pub struct StateTransferMngr<V, S, NT, PL, ST>
where
    ST: StateTransferProtocol<S>,
    V: NetworkView,
{
    #[get = "pub"]
    handle: StateTransferThreadInnerHandle<V>,
    currently_running: bool,
    node: Arc<NT>,
    latest_view: V,
    phantom: PhantomData<(S, NT, PL, ST)>,
}

pub fn init_state_transfer_handles<V>() -> (
    StateTransferThreadHandle<V>,
    StateTransferThreadInnerHandle<V>,
)
where
    V: NetworkView,
{
    let (work_tx, work_rx) =
        channel::sync::new_bounded_sync(WORK_CHANNEL_SIZE, Some("State transfer Work Channel"));
    let (response_tx, response_rx) = channel::sync::new_bounded_sync(
        RESPONSE_CHANNEL_SIZE,
        Some("State Transfer Response Channel"),
    );

    (
        StateTransferThreadHandle {
            work_tx,
            response_rx,
        },
        StateTransferThreadInnerHandle {
            work_rx,
            response_tx,
        },
    )
}

impl<V, S, NT, PL, ST> StateTransferMngr<V, S, NT, PL, ST>
where
    ST: StateTransferProtocol<S>,
    V: NetworkView,
    NT: StateTransferSendNode<ST::Serialization> + RegularNetworkStub<StateSys<ST::Serialization>>,
{
    pub(crate) fn initialize_core_state_transfer(
        node: Arc<NT>,
        state_handle: StateTransferThreadInnerHandle<V>,
        view: V,
    ) -> Result<Self> {
        Ok(Self {
            handle: state_handle,
            currently_running: false,
            node,
            latest_view: view,
            phantom: Default::default(),
        })
    }

    pub fn node(&self) -> &Arc<NT> {
        &self.node
    }

    pub fn notify_of_checkpoint(&self, seq: SeqNo) {
        self.handle
            .response_tx
            .send(StateTransferProgress::CheckpointReceived(seq))
            .unwrap()
    }

    fn handle_view(&mut self, view: &V) {
        match view
            .sequence_number()
            .index(self.latest_view.sequence_number())
        {
            Either::Left(_) => {}
            Either::Right(_) => {
                self.latest_view = view.clone();
            }
        }
    }

    pub(crate) fn handle_work_message(
        &mut self,
        state_transfer: &mut ST,
        worker_message: StateTransferWorkMessage<V>,
    ) -> Result<()> {
        match worker_message {
            StateTransferWorkMessage::ShouldRequestAppState(seq, response) => {
                Self::should_request_app_state(seq, state_transfer, response);
            }
            StateTransferWorkMessage::RequestLatestState(view) => {
                self.currently_running = true;
                self.handle_view(&view);

                Self::request_latest_state(view, state_transfer);
            }
            StateTransferWorkMessage::ViewState(view) => {
                self.handle_view(&view);
            }
            StateTransferWorkMessage::Timeout(view, timeout) => {
                self.handle_view(&view);
            }
        }

        Ok(())
    }

    pub(crate) fn handle_network_message(
        &mut self,
        state_transfer: &mut ST,
        message: StoredMessage<CstM<ST::Serialization>>,
    ) -> Result<()> {
        let view = self.latest_view.clone();

        if self.currently_running {
            let result = state_transfer.process_message(view, message);

            if let Ok(st_result) = result {
                let _ = self
                    .handle
                    .response_tx
                    .send(StateTransferProgress::StateTransferProgress(st_result));
            }
        } else {
            let _ = state_transfer.handle_off_ctx_message(view, message);
        }
        Ok(())
    }

    pub fn iterate(&mut self, state_transfer: &mut ST) -> Result<IterableProtocolRes> {
        if self.currently_running {
            match state_transfer.poll()? {
                STPollResult::ReceiveMsg => Ok(IterableProtocolRes::Receive),
                STPollResult::RePoll => Ok(IterableProtocolRes::ReRun),
                STPollResult::Exec(message) => {
                    let res = state_transfer.process_message(self.latest_view.clone(), message)?;

                    let _ = self
                        .handle
                        .response_tx
                        .send(StateTransferProgress::StateTransferProgress(res));

                    Ok(IterableProtocolRes::Continue)
                }
                STPollResult::STResult(res) => {
                    let _ = self
                        .handle
                        .response_tx
                        .send(StateTransferProgress::StateTransferProgress(res));

                    Ok(IterableProtocolRes::Continue)
                }
            }
        } else {
            Ok(IterableProtocolRes::Continue)
        }
    }

    fn request_latest_state(view: V, state_transfer: &mut ST) {
        let _ = state_transfer.request_latest_state(view);
    }

    fn should_request_app_state(
        seq: SeqNo,
        state_transfer: &mut ST,
        result: OneShotTx<ExecutionResult>,
    ) {
        let appstate_res = state_transfer.handle_app_state_requested(seq);

        if let Ok(appstate) = appstate_res {
            let _ = result.send(appstate);
        } else {
            let _ = result.send(ExecutionResult::Nil);
        }
    }
}

impl<V> StateTransferThreadHandle<V>
where
    V: NetworkView,
{
    pub fn send_work_message(&self, msg: StateTransferWorkMessage<V>) {
        let _ = self.work_tx.send(msg);
    }

    pub fn receive_state_transfer_update(&self) -> StateTransferProgress {
        self.response_rx.recv().unwrap()
    }

    pub fn try_recv_state_transfer_update(&self) -> Option<StateTransferProgress> {
        match self.response_rx.try_recv() {
            Ok(progress) => Some(progress),
            Err(_) => None,
        }
    }
}

impl<V> Clone for StateTransferThreadHandle<V>
where
    V: NetworkView,
{
    fn clone(&self) -> Self {
        Self {
            work_tx: self.work_tx.clone(),
            response_rx: self.response_rx.clone(),
        }
    }
}
