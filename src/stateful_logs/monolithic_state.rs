use std::path::Path;
use std::sync::Arc;

use atlas_common::channel;
use atlas_common::error::*;
use atlas_common::globals::ReadOnly;
use atlas_common::ordering::SeqNo;
use atlas_common::persistentdb::KVDB;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::loggable::{OrderProtocolLogHelper, PProof};
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_core::ordering_protocol::{
    BatchedDecision, DecisionAD, DecisionMetadata, ProtocolMessage, ShareableMessage,
};
use atlas_core::persistent_log::{
    OperationMode, OrderingProtocolLog, PersistableStateTransferProtocol,
};
use atlas_logging_core::decision_log::serialize::DecisionLogMessage;
use atlas_logging_core::decision_log::{
    DecLog, DecLogMetadata, DecisionLogPersistenceHelper, LoggingDecision,
};
use atlas_logging_core::persistent_log::PersistentDecisionLog;
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_application::state::monolithic_state::MonolithicState;
use atlas_smr_core::exec::WrappedExecHandle;
use atlas_smr_core::persistent_log::MonolithicStateLog;
use atlas_smr_core::state_transfer::networking::serialize::StateTransferMessage;
use atlas_smr_core::state_transfer::Checkpoint;
use atlas_smr_core::SMRReq;

use crate::worker::monolithic_worker::{
    read_mon_state, MonStatePersistentLogWorker, PersistentMonolithicStateHandle,
    PersistentMonolithicStateStub,
};
use crate::worker::{
    PersistentLogWorker, PersistentLogWorkerHandle, PersistentLogWriteStub, COLUMN_FAMILY_OTHER,
    COLUMN_FAMILY_PROOFS,
};
use crate::{PersistentLog, PersistentLogMode, PersistentLogModeTrait};

/// The persistent log handle to the worker for the monolithic state persistency log
pub struct MonStatePersistentLog<S, D, OPM, POPT, LS, STM>
where
    S: MonolithicState,
    D: ApplicationData,
    OPM: OrderingProtocolMessage<SMRReq<D>>,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM>,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT>,
    STM: StateTransferMessage,
{
    request_tx: Arc<PersistentMonolithicStateHandle<S>>,

    inner_log: PersistentLog<SMRReq<D>, OPM, POPT, LS, STM>,
}

/// The message containing the information necessary to persist the most recently received
/// Monolithic state
pub struct MonolithicStateMessage<S: MonolithicState> {
    pub(crate) checkpoint: Arc<ReadOnly<Checkpoint<S>>>,
}

/// This stupid amount of generics is because we basically interact with all of the
/// protocols in the persistent log, so we have to receive all of it
pub fn initialize_mon_persistent_log<S, D, K, T, OPM, POPT, LS, STM, PS, PSP, DLPH>(
    executor: WrappedExecHandle<D::Request>,
    db_path: K,
) -> Result<MonStatePersistentLog<S, D, OPM, POPT, LS, STM>>
where
    S: MonolithicState + 'static,
    K: AsRef<Path>,
    T: PersistentLogModeTrait,
    D: ApplicationData,
    OPM: OrderingProtocolMessage<SMRReq<D>> + 'static,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM> + 'static,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT> + 'static,
    STM: StateTransferMessage + 'static,
    PS: OrderProtocolLogHelper<SMRReq<D>, OPM, POPT>,
    PSP: PersistableStateTransferProtocol + Send + 'static,
    DLPH: DecisionLogPersistenceHelper<SMRReq<D>, OPM, POPT, LS> + 'static,
{
    MonStatePersistentLog::init_mon_log::<K, T, PS, PSP, DLPH>(executor, db_path)
}

impl<S, D, OPM, POPT, LS, STM> MonStatePersistentLog<S, D, OPM, POPT, LS, STM>
where
    S: MonolithicState + 'static,
    D: ApplicationData,
    OPM: OrderingProtocolMessage<SMRReq<D>> + 'static,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM> + 'static,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT> + 'static,
    STM: StateTransferMessage + 'static,
{
    fn init_mon_log<K, T, POS, PSP, DLPH>(
        executor: WrappedExecHandle<D::Request>,
        db_path: K,
    ) -> Result<Self>
    where
        K: AsRef<Path>,
        T: PersistentLogModeTrait,
        POS: OrderProtocolLogHelper<SMRReq<D>, OPM, POPT>,
        PSP: PersistableStateTransferProtocol + Send + 'static,
        DLPH: DecisionLogPersistenceHelper<SMRReq<D>, OPM, POPT, LS> + 'static,
    {
        let mut message_types = POS::message_types();

        let mut prefixes = vec![COLUMN_FAMILY_OTHER, COLUMN_FAMILY_PROOFS];

        prefixes.append(&mut message_types);

        let log_mode = T::init_persistent_log(executor);

        let mut response_txs = vec![];

        match &log_mode {
            PersistentLogMode::Strict(handle) => response_txs.push(handle.logger_tx().clone()),
            _ => {}
        }

        let kvdb = KVDB::new(db_path, prefixes)?;

        let (tx, rx) =
            channel::sync::new_bounded_sync(1024, Some("Mon. State Persistent Log Work Handle"));

        let worker = PersistentLogWorker::<SMRReq<D>, OPM, POPT, LS, PSP, POS, DLPH>::new(
            rx,
            response_txs,
            kvdb.clone(),
        );

        let (state_tx, state_rx) =
            channel::sync::new_bounded_sync(10, Some("Mon. State Persistent Log message"));

        let worker =
            MonStatePersistentLogWorker::<S, SMRReq<D>, OPM, POPT, LS, POS, PSP, DLPH>::new(
                state_rx,
                worker,
                kvdb.clone(),
            );

        match &log_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                std::thread::Builder::new()
                    .name(format!("Persistent log Worker #1"))
                    .spawn(move || {
                        worker.work();
                    })
                    .unwrap();
            }
            _ => {}
        }

        let persistent_log_write_stub = PersistentLogWriteStub { tx };

        let worker_handle = Arc::new(PersistentLogWorkerHandle::new(vec![
            persistent_log_write_stub,
        ]));

        let init_log = PersistentLog {
            persistency_mode: log_mode,
            worker_handle,
            p: Default::default(),
            db: kvdb,
        };

        let persistent_mon_write_stub = PersistentMonolithicStateStub { tx: state_tx };

        let worker_handle = Arc::new(PersistentMonolithicStateHandle::new(vec![
            persistent_mon_write_stub,
        ]));

        Ok(Self {
            request_tx: worker_handle,
            inner_log: init_log,
        })
    }
}

impl<S, D, OPM, POPT, LS, STM> MonolithicStateLog<S>
    for MonStatePersistentLog<S, D, OPM, POPT, LS, STM>
where
    S: MonolithicState + 'static,
    D: ApplicationData + 'static,
    OPM: OrderingProtocolMessage<SMRReq<D>> + 'static,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM> + 'static,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT> + 'static,
    STM: StateTransferMessage + 'static,
{
    #[inline]
    fn read_checkpoint(&self) -> Result<Option<Checkpoint<S>>> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => {
                read_mon_state(&self.inner_log.db)
            }
            PersistentLogMode::None => Ok(None),
        }
    }

    #[inline]
    fn write_checkpoint(
        &self,
        write_mode: OperationMode,
        checkpoint: Arc<ReadOnly<Checkpoint<S>>>,
    ) -> Result<()> {
        match self.inner_log.persistency_mode {
            PersistentLogMode::Strict(_) | PersistentLogMode::Optimistic => match write_mode {
                OperationMode::NonBlockingSync(_callback) => {
                    self.request_tx.queue_state(checkpoint)?;
                }
                OperationMode::BlockingSync => {
                    todo!()
                }
            },
            PersistentLogMode::None => {}
        }

        Ok(())
    }
}

impl<S, D, OPM, POPT, LS, STM> OrderingProtocolLog<SMRReq<D>, OPM>
    for MonStatePersistentLog<S, D, OPM, POPT, LS, STM>
where
    S: MonolithicState + 'static,
    D: ApplicationData + 'static,
    OPM: OrderingProtocolMessage<SMRReq<D>> + 'static,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM> + 'static,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT> + 'static,
    STM: StateTransferMessage + 'static,
{
    #[inline]
    fn write_committed_seq_no(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()> {
        self.inner_log.write_committed_seq_no(write_mode, seq)
    }

    #[inline]
    fn write_message(
        &self,
        write_mode: OperationMode,
        msg: ShareableMessage<ProtocolMessage<SMRReq<D>, OPM>>,
    ) -> Result<()> {
        self.inner_log.write_message(write_mode, msg)
    }

    #[inline]
    fn write_decision_metadata(
        &self,
        write_mode: OperationMode,
        metadata: DecisionMetadata<SMRReq<D>, OPM>,
    ) -> Result<()> {
        self.inner_log.write_decision_metadata(write_mode, metadata)
    }

    fn write_decision_additional_data(
        &self,
        write_mode: OperationMode,
        additional_data: DecisionAD<SMRReq<D>, OPM>,
    ) -> Result<()> {
        todo!()
    }

    #[inline]
    fn write_invalidate(&self, write_mode: OperationMode, seq: SeqNo) -> Result<()> {
        self.inner_log.write_invalidate(write_mode, seq)
    }
}

impl<S, D, OPM, POPT, LS, STM> PersistentDecisionLog<SMRReq<D>, OPM, POPT, LS>
    for MonStatePersistentLog<S, D, OPM, POPT, LS, STM>
where
    S: MonolithicState + 'static,
    D: ApplicationData + 'static,
    OPM: OrderingProtocolMessage<SMRReq<D>> + 'static,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM> + 'static,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT> + 'static,
    STM: StateTransferMessage + 'static,
{
    fn checkpoint_received(&self, mode: OperationMode, seq: SeqNo) -> Result<()> {
        self.inner_log.checkpoint_received(mode, seq)
    }

    fn write_proof(
        &self,
        write_mode: OperationMode,
        proof: PProof<SMRReq<D>, OPM, POPT>,
    ) -> Result<()> {
        self.inner_log.write_proof(write_mode, proof)
    }

    fn read_proof(
        &self,
        mode: OperationMode,
        seq: SeqNo,
    ) -> Result<Option<PProof<SMRReq<D>, OPM, POPT>>> {
        self.inner_log.read_proof(mode, seq)
    }

    fn read_decision_log(
        &self,
        mode: OperationMode,
    ) -> Result<Option<DecLog<SMRReq<D>, OPM, POPT, LS>>> {
        self.inner_log.read_decision_log(mode)
    }

    fn reset_log(&self, mode: OperationMode) -> Result<()> {
        self.inner_log.reset_log(mode)
    }

    fn write_decision_log(
        &self,
        mode: OperationMode,
        log: DecLog<SMRReq<D>, OPM, POPT, LS>,
    ) -> Result<()> {
        self.inner_log.write_decision_log(mode, log)
    }

    fn wait_for_full_persistence(
        &self,
        batch: BatchedDecision<SMRReq<D>>,
        decision_logging: LoggingDecision,
    ) -> Result<Option<BatchedDecision<SMRReq<D>>>> {
        self.inner_log
            .wait_for_full_persistence(batch, decision_logging)
    }

    fn write_decision_log_metadata(
        &self,
        mode: OperationMode,
        log_metadata: DecLogMetadata<SMRReq<D>, OPM, POPT, LS>,
    ) -> Result<()> {
        self.inner_log
            .write_decision_log_metadata(mode, log_metadata)
    }
}

impl<S, D, OPM, POPT, LS, STM> Clone for MonStatePersistentLog<S, D, OPM, POPT, LS, STM>
where
    S: MonolithicState + 'static,
    D: ApplicationData,
    OPM: OrderingProtocolMessage<SMRReq<D>> + 'static,
    POPT: PersistentOrderProtocolTypes<SMRReq<D>, OPM> + 'static,
    LS: DecisionLogMessage<SMRReq<D>, OPM, POPT> + 'static,
    STM: StateTransferMessage + 'static,
{
    fn clone(&self) -> Self {
        Self {
            request_tx: self.request_tx.clone(),
            inner_log: self.inner_log.clone(),
        }
    }
}
