use std::path::Path;

use atlas_common::error::*;
use atlas_core::execution::TDeterministicExecutorDecisionHandle;
use atlas_core::ordering_protocol::decision::BatchedDecision;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::OrderProtocolLogHelper;
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_core::persistent_log::{OrderingProtocolLog, PersistableStateTransferProtocol};
use atlas_logging_core::decision_log::serialize::DecisionLogMessage;
use atlas_logging_core::decision_log::DecisionLogPersistenceHelper;
use atlas_logging_core::persistent_log::PersistentDecisionLog;
use atlas_persistent_log::execution_handle::TLoggedDecisionsHandle;
use atlas_persistent_log::stateful_logs::monolithic_state::{
    initialize_mon_persistent_log, MonStatePersistentLog,
};
use atlas_persistent_log::PersistentLogModeTrait;
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_application::state::monolithic_state::MonolithicState;
use atlas_smr_core::state_transfer::networking::serialize::StateTransferMessage;
use atlas_smr_core::SMRReq;

pub trait SMRPersistentLog<D, OPM, POPT, LS>:
    OrderingProtocolLog<SMRReq<D>, OPM> + PersistentDecisionLog<SMRReq<D>, OPM, POPT, LS>
where
    D: ApplicationData,
    OPM: OrderingProtocolMessage<SMRReq<D>>,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM>,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT>,
{
    type Config;

    fn init_log<K, T, POS, PSP, DLPH, EX>(executor: EX, db_path: K) -> Result<Self>
    where
        K: AsRef<Path>,
        T: PersistentLogModeTrait,
        POS: OrderProtocolLogHelper<SMRReq<D>, OPM, POPT> + Send + 'static,
        PSP: PersistableStateTransferProtocol + Send + 'static,
        DLPH: DecisionLogPersistenceHelper<SMRReq<D>, OPM, POPT, LS> + 'static,
        EX: TLoggedDecisionsHandle<SMRReq<D>>,
        Self: Sized;
}

impl<S, D, OPM, POPT, LS, STM> SMRPersistentLog<D, OPM, POPT, LS>
    for MonStatePersistentLog<S, D, OPM, POPT, LS, STM>
where
    S: MonolithicState + 'static,
    D: ApplicationData + 'static,
    OPM: OrderingProtocolMessage<SMRReq<D>> + 'static,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM> + 'static,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT> + 'static,
    STM: StateTransferMessage + 'static,
{
    type Config = ();

    fn init_log<K, T, POS, PSP, DLPH, EX>(executor: EX, db_path: K) -> Result<Self>
    where
        K: AsRef<Path>,
        T: PersistentLogModeTrait,
        POS: OrderProtocolLogHelper<SMRReq<D>, OPM, POPT> + Send + 'static,
        PSP: PersistableStateTransferProtocol + Send + 'static,
        DLPH: DecisionLogPersistenceHelper<SMRReq<D>, OPM, POPT, LS> + 'static,
        EX: TLoggedDecisionsHandle<SMRReq<D>>,
        Self: Sized,
    {
        initialize_mon_persistent_log::<S, D, K, T, OPM, POPT, LS, STM, POS, PSP, DLPH, EX>(
            executor, db_path,
        )
    }
}

pub struct PersistentLogHandle<EX>(EX);

impl<EX> PersistentLogHandle<EX> {
    pub fn new(executor_handle: EX) -> Self {
        Self(executor_handle)
    }
}

impl<EX> Clone for PersistentLogHandle<EX>
where
    EX: Clone,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<EX, RQ> TLoggedDecisionsHandle<RQ> for PersistentLogHandle<EX>
where
    EX: TDeterministicExecutorDecisionHandle<RQ>,
{
    fn register_decisions_logged(&self, decision: BatchedDecision<RQ>) -> Result<()> {
        self.0.queue_update(decision)
    }
}
