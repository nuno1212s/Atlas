use std::path::Path;
use atlas_core::ordering_protocol::ProtocolConsensusDecision;
use atlas_core::persistent_log::{OrderingProtocolLog, PersistableOrderProtocol, PersistableStateTransferProtocol, StatefulOrderingProtocolLog, StateTransferProtocolLog};
use atlas_core::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage, StateTransferMessage};
use atlas_core::state_transfer::StatefulOrderProtocol;
use atlas_execution::serialize::SharedData;
use atlas_common::error::*;
use atlas_execution::ExecutorHandle;
use atlas_persistent_log::{PersistentLog, PersistentLogModeTrait};

pub trait SMRPersistentLog<D, OPM, SOPM>: OrderingProtocolLog<OPM> + StatefulOrderingProtocolLog<OPM, SOPM> + StateTransferProtocolLog<OPM, SOPM, D>
    where D: SharedData + 'static,
          OPM: OrderingProtocolMessage + 'static,
          SOPM: StatefulOrderProtocolMessage + 'static {
    fn init_log<K, T, POS, PSP>(executor: ExecutorHandle<D>, db_path: K) -> Result<Self>
        where
            K: AsRef<Path>,
            T: PersistentLogModeTrait,
            POS: PersistableOrderProtocol<OPM, SOPM> + Send + 'static,
            PSP: PersistableStateTransferProtocol + Send + 'static,
            Self: Sized;

    fn wait_for_proof_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>>;

    fn wait_for_batch_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>>;
}

impl<D, OPM, SOPM, STM> SMRPersistentLog<D, OPM, SOPM> for PersistentLog<D, OPM, SOPM, STM>
    where D: SharedData + 'static,
          OPM: OrderingProtocolMessage + 'static,
          SOPM: StatefulOrderProtocolMessage + 'static,
          STM: StateTransferMessage + 'static {
    fn init_log<K, T, POS, PSP>(executor: ExecutorHandle<D>, db_path: K) -> Result<Self>
        where K: AsRef<Path>, T: PersistentLogModeTrait,
              POS: PersistableOrderProtocol<OPM, SOPM> + Send + 'static,
              PSP: PersistableStateTransferProtocol + Send + 'static,
              Self: Sized {
        atlas_persistent_log::initialize_persistent_log::<D, K, T, OPM, SOPM, STM, POS, PSP>(executor, db_path)
    }

    fn wait_for_proof_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>> {
        self.wait_for_proof_persistency_and_execute(batch)
    }

    fn wait_for_batch_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>> {
        self.wait_for_batch_persistency_and_execute(batch)
    }
}