use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Instant;

use log::error;

use atlas_common::{channel, threadpool};
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_communication::{FullNetworkNode, NetworkNode};
use atlas_communication::protocol_node::ProtocolNetworkNode;
use atlas_core::log_transfer::LogTransferProtocol;
use atlas_core::ordering_protocol::reconfigurable_order_protocol::ReconfigurableOrderProtocol;
use atlas_core::ordering_protocol::stateful_order_protocol::StatefulOrderProtocol;
use atlas_core::persistent_log::{MonolithicStateLog, PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::reconfiguration_protocol::ReconfigurationProtocol;
use atlas_core::smr::networking::SMRNetworkNode;
use atlas_core::state_transfer::Checkpoint;
use atlas_core::state_transfer::monolithic_state::MonolithicStateTransfer;
use atlas_execution::app::Application;
use atlas_execution::state::monolithic_state::{AppStateMessage, digest_state, InstallStateMessage};
use atlas_execution::state::monolithic_state::MonolithicState;
use atlas_metrics::metrics::metric_duration;

use crate::config::MonolithicStateReplicaConfig;
use crate::executable::monolithic_executor::MonolithicExecutor;
use crate::executable::ReplicaReplier;
use crate::metric::RUN_LATENCY_TIME_ID;
use crate::persistent_log::SMRPersistentLog;
use crate::server::client_replier::Replier;
use crate::server::Replica;

/// Replica type made to handle monolithic states and executors
pub struct MonReplica<RP, S, A, OP, ST, LT, NT, PL>
    where RP: ReconfigurationProtocol + 'static,
          S: MonolithicState + 'static,
          A: Application<S> + Send + 'static,
          OP: StatefulOrderProtocol<A::AppData, NT, PL> + PersistableOrderProtocol<A::AppData, OP::Serialization, OP::StateSerialization> + ReconfigurableOrderProtocol<RP::Serialization> + 'static,
          ST: MonolithicStateTransfer<S, NT, PL> + PersistableStateTransferProtocol + 'static,
          LT: LogTransferProtocol<A::AppData, OP, NT, PL> + 'static,
          PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::StateSerialization> + 'static + MonolithicStateLog<S>, {
    p: PhantomData<A>,
    /// The inner replica object, responsible for the general replica things
    inner_replica: Replica<RP, S, A::AppData, OP, ST, LT, NT, PL>,

    state_tx: ChannelSyncTx<InstallStateMessage<S>>,
    checkpoint_rx: ChannelSyncRx<AppStateMessage<S>>,
    digested_state: (ChannelSyncTx<Arc<ReadOnly<Checkpoint<S>>>>, ChannelSyncRx<Arc<ReadOnly<Checkpoint<S>>>>),
    /// State transfer protocols
    state_transfer_protocol: ST,
}

impl<RP, S, A, OP, ST, LT, NT, PL> MonReplica<RP, S, A, OP, ST, LT, NT, PL>
    where
        RP: ReconfigurationProtocol + 'static,
        S: MonolithicState + 'static,
        A: Application<S> + Send + 'static,
        OP: StatefulOrderProtocol<A::AppData, NT, PL> + PersistableOrderProtocol<A::AppData, OP::Serialization, OP::StateSerialization> + ReconfigurableOrderProtocol<RP::Serialization> + Send + 'static,
        LT: LogTransferProtocol<A::AppData, OP, NT, PL> + 'static,
        ST: MonolithicStateTransfer<S, NT, PL> + PersistableStateTransferProtocol + Send + 'static,
        PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::StateSerialization> + MonolithicStateLog<S> + 'static,
        NT: SMRNetworkNode<RP::InformationProvider, RP::Serialization, A::AppData, OP::Serialization, ST::Serialization, LT::Serialization> + 'static, {
    pub async fn bootstrap(cfg: MonolithicStateReplicaConfig<RP, S, A, OP, ST, LT, NT, PL>) -> Result<Self> {
        let MonolithicStateReplicaConfig {
            service,
            replica_config,
            st_config
        } = cfg;

        let (executor_handle, executor_receiver) = MonolithicExecutor::<S, A, NT>::init_handle();

        let inner_replica = Replica::<RP, S, A::AppData, OP, ST, LT, NT, PL>::bootstrap(replica_config, executor_handle.clone()).await?;

        let node = inner_replica.node.clone();

        //CURRENTLY DISABLED, USING THREADPOOL INSTEAD
        let reply_handle = Replier::new(NetworkNode::id(&*node), node.clone());

        let (state_tx, checkpoint_rx) =
            MonolithicExecutor::init::<ReplicaReplier>
                (reply_handle, executor_receiver, None, service, node.clone())?;


        let state_transfer_protocol = ST::initialize(st_config, inner_replica.timeouts.clone(),
                                                     node.clone(), inner_replica.persistent_log.clone(),
                                                     state_tx.clone())?;

        let digest_app_state = channel::new_bounded_sync(5);

        let view = inner_replica.ordering_protocol.view();

        let mut replica = Self {
            p: Default::default(),
            inner_replica,
            state_tx,
            checkpoint_rx,
            digested_state: digest_app_state,
            state_transfer_protocol,
        };

        replica.state_transfer_protocol.request_latest_state(view)?;

        Ok(replica)
    }

    pub fn run(&mut self) -> Result<()> {
        let mut last_loop = Instant::now();

        loop {
            self.receive_checkpoints()?;
            self.receive_digested_checkpoints()?;

            self.inner_replica.run(&mut self.state_transfer_protocol)?;

            metric_duration(RUN_LATENCY_TIME_ID, last_loop.elapsed());

            last_loop = Instant::now();
        }
    }

    fn receive_checkpoints(&mut self) -> Result<()> {
        while let Ok(checkpoint) = self.checkpoint_rx.try_recv() {
            self.execution_finished_with_appstate(checkpoint.seq(), checkpoint.into_state())?;
        }

        Ok(())
    }

    fn receive_digested_checkpoints(&mut self) -> Result<()> {
        while let Ok(checkpoint) = self.digested_state.1.try_recv() {
            self.state_transfer_protocol.handle_state_received_from_app(self.inner_replica.ordering_protocol.view(), checkpoint.clone())?;
            self.inner_replica.ordering_protocol.checkpointed(checkpoint.sequence_number())?;
        }

        Ok(())
    }

    fn execution_finished_with_appstate(&mut self, seq: SeqNo, appstate: S) -> Result<()> {
        let return_tx = self.digested_state.0.clone();

        // Digest the app state before passing it on to the ordering protocols
        threadpool::execute(move || {
            let result = digest_state(&appstate);

            match result {
                Ok(digest) => {
                    let checkpoint = Checkpoint::new(seq, appstate, digest);

                    return_tx.send(checkpoint).unwrap();
                }
                Err(error) => {
                    error!("Failed to serialize and digest application state: {:?}", error)
                }
            }
        });

        Ok(())
    }
}