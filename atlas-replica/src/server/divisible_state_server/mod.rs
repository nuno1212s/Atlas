use std::marker::PhantomData;
use std::time::Instant;

use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::ordering::Orderable;
use atlas_communication::FullNetworkNode;
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_core::log_transfer::LogTransferProtocol;
use atlas_core::ordering_protocol::reconfigurable_order_protocol::ReconfigurableOrderProtocol;
use atlas_core::ordering_protocol::stateful_order_protocol::StatefulOrderProtocol;
use atlas_core::persistent_log::{DivisibleStateLog, PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::reconfiguration_protocol::ReconfigurationProtocol;
use atlas_core::smr::networking::SMRNetworkNode;
use atlas_core::state_transfer::divisible_state::DivisibleStateTransfer;
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

pub struct DivStReplica<RP, S, A, OP, ST, LT, NT, PL>
    where RP: ReconfigurationProtocol + 'static,
          S: DivisibleState + 'static,
          A: Application<S> + Send + 'static,
          OP: StatefulOrderProtocol<A::AppData, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + ReconfigurableOrderProtocol<RP::Serialization> + 'static,
          ST: DivisibleStateTransfer<S, NT, PL> + PersistableStateTransferProtocol + 'static,
          LT: LogTransferProtocol<A::AppData, OP, NT, PL> + 'static,
          PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::StateSerialization> + 'static + DivisibleStateLog<S>,
{
    p: PhantomData<A>,
    /// The inner replica object, responsible for the general replica things
    inner_replica: Replica<RP, S, A::AppData, OP, ST, LT, NT, PL>,

    state_tx: ChannelSyncTx<InstallStateMessage<S>>,
    checkpoint_rx: ChannelSyncRx<AppStateMessage<S>>,
    /// State transfer protocols
    state_transfer_protocol: ST,
}

impl<RP, S, A, OP, ST, LT, NT, PL> DivStReplica<RP, S, A, OP, ST, LT, NT, PL> where
    RP: ReconfigurationProtocol + 'static,
    S: DivisibleState + Send + 'static,
    A: Application<S> + Send + 'static,
    OP: StatefulOrderProtocol<A::AppData, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + ReconfigurableOrderProtocol<RP::Serialization> + Send + 'static,
    LT: LogTransferProtocol<A::AppData, OP, NT, PL> + 'static,
    ST: DivisibleStateTransfer<S, NT, PL> + PersistableStateTransferProtocol + Send + 'static,
    PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::StateSerialization> + DivisibleStateLog<S> + 'static,
    NT: SMRNetworkNode<RP::InformationProvider, RP::Serialization, A::AppData, OP::Serialization, ST::Serialization, LT::Serialization> + 'static, {
    pub async fn bootstrap(cfg: DivisibleStateReplicaConfig<RP, S, A, OP, ST, LT, NT, PL>) -> Result<Self> {
        let DivisibleStateReplicaConfig {
            service, replica_config, st_config
        } = cfg;

        let (executor_handle, executor_receiver) = DivisibleStateExecutor::<S, A, NT>::init_handle();

        let inner_replica = Replica::<RP, S, A::AppData, OP, ST, LT, NT, PL>::bootstrap(replica_config, executor_handle.clone()).await?;

        let node = inner_replica.node.clone();

        //CURRENTLY DISABLED, USING THREADPOOL INSTEAD
        let reply_handle = Replier::new(ProtocolNetworkNode::id(&*node), node.clone());

        let (state_tx, checkpoint_rx) =
            DivisibleStateExecutor::init::<ReplicaReplier>
                (reply_handle, executor_receiver, None, service, node.clone())?;

        let state_transfer_protocol = ST::initialize(st_config, inner_replica.timeouts.clone(),
                                                     node.clone(), inner_replica.persistent_log.clone(),
                                                     state_tx.clone())?;

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

            let current_view = self.inner_replica.ordering_protocol.view();

            self.state_transfer_protocol.handle_state_received_from_app(current_view, descriptor, state_parts)?;

            self.inner_replica.ordering_protocol.checkpointed(seq_no)?;
        }

        Ok(())
    }
}