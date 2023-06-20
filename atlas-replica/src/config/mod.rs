use std::marker::PhantomData;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::config::NodeConfig;
use atlas_communication::Node;
use atlas_execution::app::{Application};
use atlas_core::ordering_protocol::OrderingProtocol;
use atlas_core::persistent_log::{DivisibleStateLog, MonolithicStateLog, PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::serialize::{ServiceMsg, StateTransferMessage};
use atlas_core::state_transfer::{StateTransferProtocol};
use atlas_core::state_transfer::divisible_state::DivisibleStateTransfer;
use atlas_core::state_transfer::log_transfer::{LogTransferProtocol, StatefulOrderProtocol};
use atlas_core::state_transfer::monolithic_state::MonolithicStateTransfer;
use atlas_execution::serialize::ApplicationData;
use atlas_execution::state::divisible_state::DivisibleState;
use atlas_execution::state::monolithic_state::MonolithicState;
use atlas_persistent_log::PersistentLog;
use crate::persistent_log::SMRPersistentLog;

pub struct MonolithicStateReplicaConfig<S, A, OP, ST, LT, NT, PL>
    where S: MonolithicState + 'static,
          A: Application<S> + 'static,
          OP: StatefulOrderProtocol<A::AppData, NT, PL> + 'static + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization>,
          ST: MonolithicStateTransfer<S, NT, PL> + 'static + PersistableStateTransferProtocol,
          LT: LogTransferProtocol<A::AppData, OP, NT, PL> + 'static,
          NT: Node<ServiceMsg<A::AppData, OP::Serialization, ST::Serialization, LT::Serialization>>,
          PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::StateSerialization> + MonolithicStateLog<S> {
    /// The application logic.
    pub service: A,

    pub replica_config: ReplicaConfig<S, A::AppData, OP, ST, LT, NT, PL>,

    /// The configuration for the State transfer protocol
    pub st_config: ST::Config,
}

pub struct DivisibleStateReplicaConfig<S, A, OP, ST, LT, NT, PL>
    where S: DivisibleState + 'static,
          A: Application<S> + 'static,
          OP: StatefulOrderProtocol<A::AppData, NT, PL> + 'static + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization>,
          ST: DivisibleStateTransfer<S, NT, PL> + 'static + PersistableStateTransferProtocol,
          LT: LogTransferProtocol<A::AppData, OP, NT, PL> + 'static,
          NT: Node<ServiceMsg<A::AppData, OP::Serialization, ST::Serialization, LT::Serialization>>,
          PL: SMRPersistentLog<A::AppData, OP::Serialization, OP::StateSerialization> + DivisibleStateLog<S> {
    /// The application logic.
    pub service: A,

    pub replica_config: ReplicaConfig<S, A::AppData, OP, ST, LT, NT, PL>,

    /// The configuration for the State transfer protocol
    pub st_config: ST::Config,
}

/// Represents a configuration used to bootstrap a `Replica`.
pub struct ReplicaConfig<S, D, OP, ST, LT, NT, PL> where
    D: ApplicationData + 'static,
    OP: StatefulOrderProtocol<D, NT, PL> + 'static + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization>,
    ST: StateTransferProtocol<S, NT, PL> + 'static,
    LT: LogTransferProtocol<D, OP, NT, PL> + 'static,
    NT: Node<ServiceMsg<D, OP::Serialization, ST::Serialization, LT::Serialization>>,
    PL: SMRPersistentLog<D, OP::Serialization, OP::StateSerialization> {
    /// ID of the Node in question
    pub id: NodeId,

    /// The number of nodes in the network
    pub n: usize,
    /// The number of nodes that can fail in the network
    pub f: usize,

    ///TODO: These two values should be loaded from storage
    /// The sequence number for the current view.
    pub view: SeqNo,

    /// Next sequence number attributed to a request by
    /// the consensus layer.
    pub next_consensus_seq: SeqNo,

    /// The path to the database
    pub db_path: String,

    /// The configuration for the ordering protocol
    pub op_config: OP::Config,

    /// The configuration for the log transfer protocol
    pub lt_config: LT::Config,

    /// The configuration for the persistent log module
    pub pl_config: PL::Config,

    /// Check out the docs on `NodeConfig`.
    pub node: NT::Config,

    pub p: PhantomData<S>,
}