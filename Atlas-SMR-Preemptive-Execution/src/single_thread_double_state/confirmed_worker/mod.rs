use crate::metric::CONFIRMED_WORKER_LATENCY_ID;
use crate::single_thread_double_state::RunMode;
use crate::single_thread_double_state::comm_handles::ConfirmedWorkerSharedChannels;
use crate::single_thread_double_state::confirmed_worker::comm_handles::{
    ConfirmedChannels, ConfirmedUpdateMessage, ConfirmedWorkerHandle,
};
use crate::single_thread_double_state::confirmed_worker::confirmed_requests::ConfirmedRequestPipeline;
use crate::single_thread_double_state::state_management::{
    ConfirmedToPreemptiveMsg, PreemptiveToConfirmedMsg, StateMessage,
};
use atlas_common::channel::sync::sync_select;
use atlas_common::channel::{NoRetChannelErr, RecvError};
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::tbo_queue::TTboQueue;
use atlas_common::ordering::tbo_queue::vec_tbo_queue::VTboQueue;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::{exhaust_and_consume, quiet_unwrap, unwrap_channel};
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use atlas_metrics::metrics::metric_duration;
use atlas_smr_application::app::{Application, Request};
use atlas_smr_application::state::monolithic_state::{AppStateMessage, MonolithicState};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::repliers::ExecutorReplier;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::marker::PhantomData;
use std::sync::Arc;
use tracing::error;

pub(super) mod comm_handles;
pub(super) mod confirmed_requests;

const READ_THREAD_POOL_SIZE: usize = 4;

pub fn init_confirmed_worker<A, S, NT, T>(
    state: (SeqNo, S),
    application: Arc<A>,
    send_node: Arc<NT>,
    shared_worker_channels: ConfirmedWorkerSharedChannels<Request<A, S>, S>,
) -> ConfirmedWorkerHandle<Request<A, S>, S>
where
    A: Application<S> + 'static,
    S: MonolithicState + Sync,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    T: ExecutorReplier + 'static,
{
    let (worker_handle, worker_channels) = comm_handles::initialize_handles(shared_worker_channels);

    let worker_state = ConfirmedUpdateExecutor::<_, _, _, T> {
        application,
        send_node,
        confirmed_state: ConfirmedRequestPipeline::new(state.0, state.1),
        confirmed_channels: worker_channels,
        run_mode: RunMode::Normal,
        read_thread_pool: ThreadPoolBuilder::new()
            .num_threads(READ_THREAD_POOL_SIZE)
            .build()
            .unwrap(),
        update_queue: VTboQueue::default(),
        _phantom: PhantomData,
    };

    worker_state.spawn_worker();

    worker_handle
}

struct ConfirmedUpdateExecutor<S, A, NT, T>
where
    A: Application<S>,
{
    application: Arc<A>,
    confirmed_state: ConfirmedRequestPipeline<S>,
    send_node: Arc<NT>,
    confirmed_channels: ConfirmedChannels<Request<A, S>, S>,
    update_queue: VTboQueue<Update<Request<A, S>>>,

    read_thread_pool: ThreadPool,

    run_mode: RunMode,

    _phantom: PhantomData<T>,
}

impl<S, A, NT, T> ConfirmedUpdateExecutor<S, A, NT, T>
where
    A: Application<S>,
    S: MonolithicState + Sync,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
    T: ExecutorReplier + 'static,
{
    fn spawn_worker(mut self)
    where
        A: 'static,
        S: 'static,
    {
        std::thread::Builder::new()
            .name("Confirmed Worker Thread".to_string())
            .spawn(move || {
                self.worker();
            })
            .unwrap();
    }

    fn worker(&mut self) {
        loop {
            match self.run_mode {
                RunMode::Normal => {
                    if let Err(err) = self.run_normal_mode() {
                        error!("Confirmed worker failed with error: {:?}", err);

                        break;
                    }
                }
                RunMode::StateTransfer => {
                    if let Err(err) = self.run_state_transfer_mode() {
                        error!("Confirmed worker failed with error: {:?}", err);

                        break;
                    }
                }
            }
        }
    }

    fn run_normal_mode(&mut self) -> Result<(), NoRetChannelErr> {
        self.execute_updates();

        sync_select! {
            recv(unwrap_channel!(self.confirmed_channels.update_messages())) -> msg =>
            exhaust_and_consume!(msg.map_err(RecvError::from)?,
                self.confirmed_channels.update_messages(),
                self, handle_work_message),
            recv(unwrap_channel!(self.confirmed_channels.incoming_preemptive_msg())) -> msg =>
            exhaust_and_consume!(msg.map_err(RecvError::from)?,
                self.confirmed_channels.incoming_preemptive_msg(),
                self, handle_confirmed_update),
        }
    }

    fn run_state_transfer_mode(&mut self) -> Result<(), NoRetChannelErr> {
        let message = self.confirmed_channels.state_messages().recv()?;

        self.handle_state_install_message(message)
    }

    fn execute_and_advance(&mut self, batch: UpdateBatch<Request<A, S>>) {
        self.confirmed_state
            .execute_update(&*self.application, batch);

        self.update_queue.advance_seq();
    }

    fn execute_updates(&mut self) {
        while let Some(update) = self.update_queue.pop() {
            match update {
                Update::Update(batch) => {
                    self.execute_and_advance(batch);
                }
                Update::UpdateAndGetState(batch) => {
                    self.execute_and_advance(batch);

                    let (seq, state) = self.confirmed_state.take_state_snapshot();
                    quiet_unwrap!(
                        self.confirmed_channels
                            .state_emission_channel()
                            .send(AppStateMessage::new(seq, state))
                    );
                }
            }
        }
    }

    fn handle_work_message(
        &mut self,
        message: ConfirmedUpdateMessage<Request<A, S>>,
    ) -> Result<(), NoRetChannelErr>
    where
        S: Clone,
    {
        match message {
            ConfirmedUpdateMessage::CatchUp(catch_up) => {
                self.handle_catch_up(catch_up);
            }
            ConfirmedUpdateMessage::UpdateBatch(update_batch, send_time) => {
                metric_duration(CONFIRMED_WORKER_LATENCY_ID, send_time.elapsed());

                if let Err(err) = self.update_queue.push(Update::Update(update_batch)) {
                    error!("Failed to push confirmed update batch: {:?}", err);
                }
            }
            ConfirmedUpdateMessage::UpdateFinalizedAndGetAppstateBatch(update_batch, send_time) => {
                metric_duration(CONFIRMED_WORKER_LATENCY_ID, send_time.elapsed());

                if let Err(err) = self
                    .update_queue
                    .push(Update::UpdateAndGetState(update_batch))
                {
                    error!("Failed to push confirmed update get state batch: {:?}", err);
                }
            }
            ConfirmedUpdateMessage::ExecuteUnordered(unordered_batch) => {
                self.handle_unordered_batch(unordered_batch);
            }
            ConfirmedUpdateMessage::StateTransferAvailable => {
                self.set_run_mode(RunMode::StateTransfer);
            }
        }

        Ok(())
    }

    fn handle_confirmed_update(
        &mut self,
        update_msg: PreemptiveToConfirmedMsg<Request<A, S>>,
    ) -> Result<(), NoRetChannelErr>
    where
        S: Clone,
    {
        match update_msg {
            PreemptiveToConfirmedMsg::UpdateConfirmed(confirmed_update) => {
                if let Err(err) = self.update_queue.push(Update::Update(confirmed_update)) {
                    error!("Failed to push confirmed update: {:?}", err);
                }
            }
            PreemptiveToConfirmedMsg::UpdateConfirmedEmitAppState(confirmed_update) => {
                if let Err(err) = self
                    .update_queue
                    .push(Update::UpdateAndGetState(confirmed_update))
                {
                    error!("Failed to push confirmed update: {:?}", err);
                }
            }
            PreemptiveToConfirmedMsg::RequestStateCopy(_) => {
                let (seq_no, state) = self.confirmed_state.take_state_snapshot();

                self.confirmed_channels
                    .outgoing_msg()
                    .send(ConfirmedToPreemptiveMsg(seq_no, state))?;
            }
        }

        Ok(())
    }

    fn handle_state_install_message(
        &mut self,
        message: StateMessage<S>,
    ) -> Result<(), NoRetChannelErr> {
        let StateMessage::ConfirmedStateReceived(seq, s) = message;

        self.confirmed_state.install_state_message(seq, s.clone());
        self.update_queue.reset_with_seq(seq);

        self.set_run_mode(RunMode::Normal);

        Ok(())
    }

    fn handle_catch_up(&mut self, confirmed_batches: MaybeVec<UpdateBatch<Request<A, S>>>)
    where
        S: Clone,
    {
        for batch in confirmed_batches {
            let seq = batch.seq_no();

            let batch_replies = self
                .confirmed_state
                .execute_update(&*self.application, batch);

            T::execution_finished::<A::AppData, NT>(
                self.send_node.clone(),
                Some(seq),
                batch_replies,
            );
        }

        self.update_queue
            .advance_to_seq(self.confirmed_state.sequence_number())
            .expect("Failed to advance to seq number.");
    }

    fn handle_unordered_batch(&self, unordered_batch: UnorderedUpdateBatch<Request<A, S>>) {
        // Unordered batches should be executed on the confirmed state
        // As we only want to return confirmed information which can not be rolled bac
        let replies = self.confirmed_state.execute_read(
            &*self.application,
            unordered_batch,
            &self.read_thread_pool,
        );

        T::execution_finished::<A::AppData, NT>(self.send_node.clone(), None, replies);
    }

    fn set_run_mode(&mut self, run_mode: RunMode) {
        self.run_mode = run_mode;
    }
}

/// An update type
enum Update<R> {
    Update(UpdateBatch<R>),
    UpdateAndGetState(UpdateBatch<R>),
}
impl<R> Orderable for Update<R> {
    fn sequence_number(&self) -> SeqNo {
        match self {
            Update::Update(batch) | Update::UpdateAndGetState(batch) => batch.seq_no(),
        }
    }
}
