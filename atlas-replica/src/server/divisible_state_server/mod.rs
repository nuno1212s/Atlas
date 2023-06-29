use std::marker::PhantomData;
use std::time::Instant;
use atlas_common::error::*;
use atlas_common::channel;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::Orderable;
use atlas_communication::Node;
use atlas_core::persistent_log::{DivisibleStateLog, PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::serialize::ServiceMsg;
use atlas_core::state_transfer::divisible_state::DivisibleStateTransfer;
use atlas_core::state_transfer::log_transfer::{LogTransferProtocol, StatefulOrderProtocol};
use atlas_execution::app::Application;
use atlas_execution::state::divisible_state::{AppStateMessage, DivisibleState, InstallStateMessage};
use atlas_metrics::metrics::metric_duration;
use crate::config::DivisibleStateReplicaConfig;
use crate::executable::divisible_state_exec::DivisibleStateExecutor;
use crate::executable::ReplicaReplier;
use crate::metric::RUN_LATENCY_TIME_ID;
use crate::persistent_log::SMRPersistentLog;
use crate::server::client_replier::Replier;
use crate::server::Replica;

pub struct DivStReplica<S, A, OP, ST, LT, NT, PL>
    where S: DivisibleState + 'static,
          A: Application<S> + Send + 'static,
          OP: StatefulOrderProtocol<A::AppData, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + 'static,
          ST: DivisibleStateTransfer<S, NT, PL> + PersistableStateTransferProtocol + 'static,
          LT: LogTransferProtocol<A::AppData, OP, NT, PL> + 'static,
          PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::StateSerialization> + 'static + DivisibleStateLog<S>,
{
    p: PhantomData<A>,
    /// The inner replica object, responsible for the general replica things
    inner_replica: Replica<S, A::AppData, OP, ST, LT, NT, PL>,

    state_tx: ChannelSyncTx<InstallStateMessage<S>>,
    checkpoint_rx: ChannelSyncRx<AppStateMessage<S>>,
    /// State transfer protocols
    state_transfer_protocol: ST,
}

impl<S, A, OP, ST, LT, NT, PL> DivStReplica<S, A, OP, ST, LT, NT, PL> where
    S: DivisibleState + Send + 'static,
    A: Application<S> + Send + 'static,
    OP: StatefulOrderProtocol<A::AppData, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + Send + 'static,
    LT: LogTransferProtocol<A::AppData, OP, NT, PL> + 'static,
    ST: DivisibleStateTransfer<S, NT, PL> + PersistableStateTransferProtocol + Send + 'static,
    PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::StateSerialization> + DivisibleStateLog<S> + 'static,
    NT: Node<ServiceMsg<A::AppData, OP::Serialization, ST::Serialization, LT::Serialization>> + 'static {
    pub async fn bootstrap(cfg: DivisibleStateReplicaConfig<S, A, OP, ST, LT, NT, PL>) -> Result<Self> {
        let DivisibleStateReplicaConfig {
            service, replica_config, st_config
        } = cfg;


        let (executor_handle, executor_receiver) = DivisibleStateExecutor::<S, A, NT>::init_handle();

        let inner_replica = Replica::<S, A::AppData, OP, ST, LT, NT, PL>::bootstrap(replica_config, executor_handle.clone()).await?;

        let node = inner_replica.node.clone();

        //CURRENTLY DISABLED, USING THREADPOOL INSTEAD
        let reply_handle = Replier::new(node.id(), node.clone());

        let (state_tx, checkpoint_rx) =
            DivisibleStateExecutor::init::<OP::Serialization, ST::Serialization, LT::Serialization, ReplicaReplier>
                (reply_handle, executor_receiver, None, service, inner_replica.node.clone())?;

        let state_transfer_protocol = ST::initialize(st_config, inner_replica.timeouts.clone(),
                                                     inner_replica.node.clone(),
                                                     inner_replica.persistent_log.clone(), state_tx.clone())?;

        let view = inner_replica.ordering_protocol.view();

        let mut replica = Self {
            p: Default::default(),
            inner_replica,
            state_tx,
            checkpoint_rx,
            state_transfer_protocol,
        };

        replica.state_transfer_protocol.request_latest_state(view)?;

        Ok(replica)
    }

    pub fn run(&mut self) -> Result<()> {
        let mut last_loop = Instant::now();

        loop {
            self.receive_checkpoints()?;

            self.inner_replica.run(&mut self.state_transfer_protocol)?;

            metric_duration(RUN_LATENCY_TIME_ID, last_loop.elapsed());

            last_loop = Instant::now();
        }
    }

    fn receive_checkpoints(&mut self) -> Result<()> {
        while let Ok(checkpoint) = self.checkpoint_rx.try_recv() {
            let seq_no = checkpoint.sequence_number();

            let (descriptor, state_parts) = checkpoint.into_state();

            self.state_transfer_protocol.handle_state_received_from_app(descriptor, state_parts)?;
            self.inner_replica.ordering_protocol.checkpointed(seq_no)?;
        }

        Ok(())
    }
}