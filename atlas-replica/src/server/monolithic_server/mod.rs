use std::sync::Arc;
use log::error;
use atlas_common::error::*;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_common::ordering::SeqNo;
use atlas_common::{channel, threadpool};
use atlas_common::globals::ReadOnly;
use atlas_communication::Node;
use atlas_core::persistent_log::{PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::serialize::ServiceMsg;
use atlas_core::state_transfer::log_transfer::{LogTransferProtocol, StatefulOrderProtocol};
use atlas_core::state_transfer::monolithic_state::MonolithicStateTransfer;
use atlas_core::state_transfer::{Checkpoint, StateTransferProtocol};
use atlas_execution::serialize::SharedData;
use atlas_execution::state::monolithic_state::{AppStateMessage, InstallStateMessage};
use atlas_execution::state::monolithic_state::MonolithicState;
use crate::config::MonolithicStateReplicaConfig;
use crate::executable::monolithic_executor::MonolithicExecutor;
use crate::persistent_log::SMRPersistentLog;
use crate::server::client_replier::Replier;
use crate::server::Replica;

/// Replica type made to handle monolithic states and executors
pub struct MonReplica<S, A, OP, ST, LT, NT, PL>
    where S: MonolithicState + 'static,
          A: SharedData + 'static,
          OP: StatefulOrderProtocol<A, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + 'static,
          ST: MonolithicStateTransfer<A, NT, PL> + PersistableStateTransferProtocol + 'static,
          LT: LogTransferProtocol<A, OP, NT, PL> + 'static,
          PL: SMRPersistentLog<A, OP::Serialization, OP::StateSerialization> + 'static, {
    /// The inner replica object, responsible for the general replica things
    inner_replica: Replica<A, OP, ST, LT, NT, PL>,

    state_tx: ChannelSyncTx<InstallStateMessage<S>>,
    checkpoint_rx: ChannelSyncRx<AppStateMessage<S>>,
    digested_state: (ChannelSyncTx<Arc<ReadOnly<Checkpoint<S>>>>, ChannelSyncRx<Arc<ReadOnly<Checkpoint<S>>>>),
    /// State transfer protocols
    state_transfer_protocol: ST,
}

impl<S, A, OP, ST, LT, NT, PL> MonReplica<S, A, OP, ST, LT, NT, PL>
    where
        S: MonolithicState + 'static,
        A: SharedData + 'static,
        OP: StatefulOrderProtocol<A, NT, PL> + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization> + 'static,
        ST: MonolithicStateTransfer<A, NT, PL> + PersistableStateTransferProtocol + 'static,
        LT: LogTransferProtocol<A, OP, NT, PL> + 'static,
        PL: SMRPersistentLog<A, OP::Serialization, OP::StateSerialization> + 'static,
        NT: Node<ServiceMsg<A, OP::Serialization, ST::Serialization, LT::Serialization>> {
    pub async fn bootstrap(cfg: MonolithicStateReplicaConfig<S, A, OP, ST, LT, PL, NT>) -> Result<Self> {
        let MonolithicStateReplicaConfig {
            service,
            replica_config,
            st_config
        } = cfg;

        let (executor_handle, executor_receiver) = MonolithicExecutor::init_handle();

        let inner_replica = Replica::bootstrap(replica_config, executor_handle.clone()).await?;

        //CURRENTLY DISABLED, USING THREADPOOL INSTEAD
        let reply_handle = Replier::new(node.id(), node.clone());

        let (state_tx, checkpoint_rx) = MonolithicExecutor::init(reply_handle, executor_receiver, None, service, inner_replica.node.clone())?;

        let state_transfer_protocol = ST::initialize(st_config, inner_replica.timeouts.clone(),
                                                     inner_replica.node.clone(),
                                                     inner_replica.persistent_log.clone(), state_tx.clone())?;

        let digest_app_state = channel::new_bounded_sync(5);

        let mut replica = Self {
            inner_replica,
            state_tx,
            checkpoint_rx,
            digested_state: digest_app_state,
            state_transfer_protocol,
        };

        replica.state_transfer_protocol.request_latest_state()?;

        Ok(replica)
    }

    fn receive_checkpoints(&mut self) {
        while let Ok(checkpoint) = self.checkpoint_rx.try_recv() {
            self.execution_finished_with_appstate(checkpoint.seq(), checkpoint.appstate)?;
        }
    }

    fn receive_digested_checkpoints(&mut self) -> Result<()>{
        while let Ok(checkpoint) = self.digested_state.1.try_recv() {
            self.state_transfer_protocol.handle_state_received_from_app(checkpoint)?;
        }

        Ok(())
    }

    fn execution_finished_with_appstate(&mut self, seq: SeqNo, appstate: S) -> Result<()> {
        let return_tx = self.digested_state.0.clone();

// Digest the app state before passing it on to the ordering protocols
        threadpool::execute(move || {
            let result = atlas_execution::serialize::digest_state::<S>(&appstate);

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