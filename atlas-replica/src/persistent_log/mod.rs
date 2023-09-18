use std::path::Path;
use atlas_core::ordering_protocol::ProtocolConsensusDecision;
use atlas_core::persistent_log::{OrderingProtocolLog, PersistableOrderProtocol, PersistableStateTransferProtocol, StatefulOrderingProtocolLog};
use atlas_execution::serialize::ApplicationData;
use atlas_common::error::*;
use atlas_core::ordering_protocol::networking::serialize::{OrderingProtocolMessage, PermissionedOrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_core::state_transfer::networking::serialize::StateTransferMessage;
use atlas_execution::ExecutorHandle;
use atlas_execution::state::monolithic_state::MonolithicState;
use atlas_persistent_log::{MonStatePersistentLog, PersistentLog, PersistentLogModeTrait};

pub trait SMRPersistentLog<D, OPM, SOPM, POP>: OrderingProtocolLog<D, OPM> + StatefulOrderingProtocolLog<D, OPM, SOPM, POP>
    where D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static {
    type Config;

    fn init_log<K, T, POS, PSP>(executor: ExecutorHandle<D>, db_path: K) -> Result<Self>
        where
            K: AsRef<Path>,
            T: PersistentLogModeTrait,
            POS: PersistableOrderProtocol<D, OPM, SOPM> + Send + 'static,
            PSP: PersistableStateTransferProtocol + Send + 'static,
            Self: Sized;

    fn wait_for_proof_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>>;

    fn wait_for_batch_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>>;
}

impl<S, D, OPM, SOPM, POP, STM> SMRPersistentLog<D, OPM, SOPM, POP> for MonStatePersistentLog<S, D, OPM, SOPM, POP, STM>
    where S: MonolithicState + 'static,
          D: ApplicationData + 'static,
          OPM: OrderingProtocolMessage<D> + 'static,
          POP: PermissionedOrderingProtocolMessage + 'static,
          SOPM: StatefulOrderProtocolMessage<D, OPM> + 'static,
          STM: StateTransferMessage + 'static {
    type Config = ();

    fn init_log<K, T, POS, PSP>(executor: ExecutorHandle<D>, db_path: K) -> Result<Self>
        where K: AsRef<Path>, T: PersistentLogModeTrait,
              POS: PersistableOrderProtocol<D, OPM, SOPM> + Send + 'static,
              PSP: PersistableStateTransferProtocol + Send + 'static,
              Self: Sized {
        atlas_persistent_log::initialize_mon_persistent_log::<S, D, K, T, OPM, SOPM, POP, STM, POS, PSP>(executor, db_path)
    }

    fn wait_for_proof_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>> {
        self.wait_for_proof_persistency_and_execute(batch)
    }

    fn wait_for_batch_persistency_and_execute(&self, batch: ProtocolConsensusDecision<D::Request>) -> Result<Option<ProtocolConsensusDecision<D::Request>>> {
        self.wait_for_batch_persistency_and_execute(batch)
    }
}