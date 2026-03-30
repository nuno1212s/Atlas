use crate::single_thread_double_state::RunMode;
use crate::single_thread_double_state::confirmed_worker::comm_handles::{
    ConfirmedChannels, ConfirmedUpdateMessage,
};
use crate::single_thread_double_state::confirmed_worker::confirmed_requests::ConfirmedRequestPipeline;
use crate::single_thread_double_state::state_management::{
    ConfirmedToPreemptiveMsg, PreemptiveStateMessage, PreemptiveToConfirmedMsg,
};
use atlas_common::channel::sync::ChannelSyncTx;
use atlas_common::maybe_vec::MaybeVec;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::quiet_unwrap;
use atlas_core::execution::requests::{UnorderedUpdateBatch, UpdateBatch};
use atlas_smr_application::app::{Application, Request};
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_application::state::monolithic_state::{InstallStateMessage, MonolithicState};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::reply::ReplyNode;
use atlas_smr_execution::repliers::ExecutorReplier;
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::Arc;
use std::thread::Thread;

mod comm_handles;
pub(super) mod confirmed_requests;

const READ_THREAD_POOL_SIZE: usize = 4;

struct ConfirmedUpdateExecutor<S, A, NT>
where
    A: Application<S>,
{
    application: Arc<A>,
    confirmed_state: ConfirmedRequestPipeline<S>,
    send_node: Arc<NT>,
    confirmed_channels: ConfirmedChannels<Request<A, S>, S>,

    read_thread_pool: ThreadPool,

    run_mode: RunMode,
}

impl<S, A, NT> ConfirmedUpdateExecutor<S, A, NT>
where
    A: Application<S>,
    S: MonolithicState + Sync,
    NT: ReplyNode<SMRReply<A::AppData>> + 'static,
{
    fn worker<T>(&mut self)
    where
        T: ExecutorReplier + 'static,
    {
        loop {
            if let Ok(update) = self.confirmed_channels.update_messages().recv() {
                self.handle_work_message::<T>(update);
            }
        }
    }

    fn handle_work_message<T>(&mut self, message: ConfirmedUpdateMessage<Request<A, S>>)
    where
        T: ExecutorReplier + 'static,
        S: Clone,
    {
        match message {
            ConfirmedUpdateMessage::CatchUp(catch_up) => {
                self.handle_catch_up::<T>(catch_up);
            }
            ConfirmedUpdateMessage::UpdateBatch(_, _) => {}
            ConfirmedUpdateMessage::UpdateFinalizedAndGetAppstateBatch(_, _) => {}
            ConfirmedUpdateMessage::ExecuteUnordered(unordered_batch) => {
                self.handle_unordered_batch::<T>(unordered_batch);
            }
        }
    }

    fn handle_confirmed_update(&mut self, update_msg: PreemptiveToConfirmedMsg<Request<A, S>>)
    where
        S: Clone,
    {
        match update_msg {
            PreemptiveToConfirmedMsg::UpdateConfirmed(confirmed_update) => {
                self.confirmed_state
                    .execute_update(&*self.application, confirmed_update);
            }
            PreemptiveToConfirmedMsg::RequestStateCopy(_) => {
                let (seq_no, state) = self.confirmed_state.take_state_snapshot();

                quiet_unwrap!(
                    self.confirmed_channels
                        .outgoing_msg()
                        .send(ConfirmedToPreemptiveMsg(seq_no, state))
                );
            }
        }
    }

    fn handle_state_install_message(&mut self, message: InstallStateMessage<S>) {
        let seq = message.sequence_number();

        let s = message.into_state();

        self.confirmed_state.install_state_message(seq, s.clone());

        quiet_unwrap!(
            self.confirmed_channels
                .outgoing_msg()
                .send(ConfirmedToPreemptiveMsg(seq, s))
        );
    }

    fn handle_catch_up<T>(&mut self, confirmed_batches: MaybeVec<UpdateBatch<Request<A, S>>>)
    where
        T: ExecutorReplier + 'static,
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

        let (seq_no, state) = self.confirmed_state.take_state_snapshot();

        //TODO: Pass on to preemptive worker
    }

    fn handle_unordered_batch<T>(&self, unordered_batch: UnorderedUpdateBatch<Request<A, S>>)
    where
        S: MonolithicState + Sync,
        T: ExecutorReplier + 'static,
    {
        // Unordered batches should be executed on the confirmed state
        // As we only want to return confirmed information which can not be rolled bac
        let replies = self.confirmed_state.execute_read(
            &*self.application,
            unordered_batch,
            &self.read_thread_pool,
        );

        T::execution_finished::<A::AppData, NT>(self.send_node.clone(), None, replies);
    }
}

fn init_confirmed_update_executor<S, A, NT>(
    confirmed_channels: ConfirmedChannels<Request<A, S>, S>,
    state: (SeqNo, S),
    application: Arc<A>,
    send_node: Arc<NT>,
) where
    A: Application<S>,
{
    let confirmed_state = ConfirmedRequestPipeline::new(state.0, state.1);

    let confirmed_updates = ConfirmedUpdateExecutor {
        confirmed_state,
        send_node,
        application,
        read_thread_pool: ThreadPoolBuilder::new()
            .num_threads(READ_THREAD_POOL_SIZE)
            .build()
            .unwrap(),
        confirmed_channels,
        run_mode: RunMode::Normal,
    };
}
