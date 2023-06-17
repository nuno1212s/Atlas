use atlas_common::error::*;
use atlas_common::channel::{ChannelSyncRx, ChannelSyncTx};
use atlas_communication::Node;
use atlas_core::persistent_log::{PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::serialize::ServiceMsg;
use atlas_core::state_transfer::log_transfer::{LogTransferProtocol, StatefulOrderProtocol};
use atlas_core::state_transfer::monolithic_state::MonolithicStateTransfer;
use atlas_core::state_transfer::StateTransferProtocol;
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

        let inner_replica = Replica::bootstrap(replica_config, executor_handle).await?;


        let state_transfer_protocol = ST::initialize(st_config, inner_replica.timeouts.clone(),
                                                     inner_replica.node.clone(),
                                                     inner_replica.persistent_log.clone())?;

        //CURRENTLY DISABLED, USING THREADPOOL INSTEAD
        let reply_handle = Replier::new(node.id(), node.clone());

        let (state_tx, checkpoint_rx) = MonolithicExecutor::init(reply_handle, executor_receiver, None, service, inner_replica.node.clone())?;

        Ok(Self {
            inner_replica,
            state_tx,
            checkpoint_rx,
            state_transfer_protocol,
        })
    }
}