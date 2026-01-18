use std::marker::PhantomData;
use std::time::Instant;

use atlas_common::error::*;
use atlas_core::ordering_protocol::loggable::TLoggableOrderProtocol;
use atlas_core::ordering_protocol::networking::NetworkedOrderProtocolInitializer;
use atlas_core::ordering_protocol::permissioned::{
    ViewTransferProtocol, ViewTransferProtocolInitializer,
};
use atlas_core::persistent_log::PersistableStateTransferProtocol;
use atlas_core::reconfiguration_protocol::ReconfigurationProtocol;
use atlas_logging_core::decision_log::{TDecisionLog, DecisionLogInitializer};
use atlas_logging_core::log_transfer::{LogTransferProtocol, LogTransferProtocolInitializer};
use atlas_metrics::metrics::metric_duration;
use atlas_smr_application::app::Application;
use atlas_smr_application::state::divisible_state::DivisibleState;
use atlas_smr_core::execution::{TExecutor, WrappedExecHandle};
use atlas_smr_core::execution::executors::divisible_state::TDivisibleStateExecutor;
use atlas_smr_core::networking::SMRReplicaNetworkNode;
use atlas_smr_core::persistent_log::DivisibleStateLog;
use atlas_smr_core::request_pre_processing::RequestPreProcessor;
use atlas_smr_core::state_transfer::divisible_state::{
    DivisibleStateTransfer, DivisibleStateTransferInitializer,
};
use atlas_smr_core::SMRReq;

use crate::config::DivisibleStateReplicaConfig;
use crate::metric::RUN_LATENCY_TIME_ID;
use crate::persistent_log::SMRPersistentLog;
use crate::server::divisible_state_server::state_transfer::DivStateTransfer;
use crate::server::state_transfer::init_state_transfer_handles;
use crate::server::{PermissionedProtocolHandling, Replica};

mod state_transfer;

pub struct DivStReplica<RP, SE, S, A, OP, DL, ST, LT, VT, NT, PL>
where
    RP: ReconfigurationProtocol + 'static,
    S: DivisibleState + 'static,
    SE: TExecutor<A, S>,
    A: Application<S> + Send,
    OP: TLoggableOrderProtocol<SMRReq<A::AppData>>,
    LT: LogTransferProtocol<SMRReq<A::AppData>, OP, DL>,
    DL: TDecisionLog<SMRReq<A::AppData>, OP>,
    VT: ViewTransferProtocol<OP>,
    ST: DivisibleStateTransfer<S> + PersistableStateTransferProtocol,
    PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>
        + 'static
        + DivisibleStateLog<S>,
    NT: SMRReplicaNetworkNode<
            RP::InformationProvider,
            RP::Serialization,
            A::AppData,
            OP::Serialization,
            LT::Serialization,
            VT::Serialization,
            ST::Serialization,
        > + 'static,
{
    p: PhantomData<fn() -> (A, SE)>,
    /// The inner replica object, responsible for the general replica things
    inner_replica: Replica<RP, S, A::AppData, OP, DL, ST, LT, VT, NT, PL, WrappedExecHandle<SE::ExecutionHandle>>,
}

impl<RP, SE, S, A, OP, DL, ST, LT, VT, NT, PL>
    DivStReplica<RP, SE, S, A, OP, DL, ST, LT, VT, NT, PL>
where
    RP: ReconfigurationProtocol + 'static,
    SE: TDivisibleStateExecutor<A, S, NT::ApplicationNode> + 'static,
    S: DivisibleState + Send + 'static,
    A: Application<S> + Send + 'static,
    OP: TLoggableOrderProtocol<SMRReq<A::AppData>> + Send + 'static,
    DL: TDecisionLog<SMRReq<A::AppData>, OP> + 'static,
    LT: LogTransferProtocol<SMRReq<A::AppData>, OP, DL> + 'static,
    VT: ViewTransferProtocol<OP> + 'static,
    ST: DivisibleStateTransfer<S> + PersistableStateTransferProtocol + Send + 'static,
    PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::PersistableTypes, DL::LogSerialization>
        + DivisibleStateLog<S>
        + 'static,
    NT: SMRReplicaNetworkNode<
            RP::InformationProvider,
            RP::Serialization,
            A::AppData,
            OP::Serialization,
            LT::Serialization,
            VT::Serialization,
            ST::Serialization,
        > + 'static,
{
    pub async fn bootstrap(
        cfg: DivisibleStateReplicaConfig<RP, S, A, OP, DL, ST, LT, VT, NT, PL>,
    ) -> Result<Self>
    where
        OP: NetworkedOrderProtocolInitializer<
            SMRReq<A::AppData>,
            RequestPreProcessor<SMRReq<A::AppData>>,
            NT::ProtocolNode,
        >,
        VT: ViewTransferProtocolInitializer<OP, NT::ProtocolNode>,
        LT: LogTransferProtocolInitializer<
            SMRReq<A::AppData>,
            OP,
            DL,
            PL,
            WrappedExecHandle<SE::ExecutionHandle>,
            NT::ProtocolNode,
        >,
        DL: DecisionLogInitializer<SMRReq<A::AppData>, OP, PL, WrappedExecHandle<SE::ExecutionHandle>>,
        ST: DivisibleStateTransferInitializer<S, NT::StateTransferNode, PL>,
        
    {
        let DivisibleStateReplicaConfig {
            service,
            replica_config,
            st_config,
        } = cfg;

        let (handle, inner_handle) = init_state_transfer_handles();

        let executor_handle = SE::init_handle();

        let wrapped_executor = WrappedExecHandle(executor_handle.clone());

        let inner_replica = Replica::bootstrap(replica_config, wrapped_executor, handle)
        .await?;

        let node = inner_replica.node.clone();

        let (state_tx, checkpoint_rx) =
            SE::init(executor_handle, None, service, node.app_node().clone())?;

        DivStateTransfer
            ::<<Replica<RP, S, A::AppData, OP, DL, ST, LT, VT, NT, PL, WrappedExecHandle<SE::ExecutionHandle>> as PermissionedProtocolHandling<A::AppData, VT, OP, NT>>::View,
            S, NT::StateTransferNode, PL, ST>
        ::init_state_transfer_thread(state_tx, checkpoint_rx, st_config,
                                     node.state_transfer_node().clone(),
                                     inner_replica.timeouts.gen_mod_handle_with_name(ST::mod_name()),
                                     inner_replica.persistent_log.clone(),
                                     inner_handle, inner_replica.view());

        let mut replica = Self {
            p: Default::default(),
            inner_replica,
        };

        replica.bootstrap_protocols()?;

        Ok(replica)
    }

    /// Bootstrap our SMR protocols in order to start
    fn bootstrap_protocols(&mut self) -> Result<()> {
        self.inner_replica.bootstrap_protocols()
    }

    /// Run the main replica thread
    pub fn run(&mut self) -> Result<()> {
        let mut last_loop = Instant::now();

        loop {
            self.inner_replica.iterate()?;

            metric_duration(RUN_LATENCY_TIME_ID, last_loop.elapsed());

            last_loop = Instant::now();
        }
    }
}
