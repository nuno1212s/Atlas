use std::marker::PhantomData;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_communication::config::NodeConfig;
use atlas_communication::Node;
use atlas_execution::app::Service;
use atlas_core::ordering_protocol::OrderingProtocol;
use atlas_core::persistent_log::{PersistableOrderProtocol, PersistableStateTransferProtocol};
use atlas_core::serialize::ServiceMsg;
use atlas_core::state_transfer::{StatefulOrderProtocol, StateTransferProtocol};
use atlas_persistent_log::PersistentLog;
use crate::persistent_log::SMRPersistentLog;

/// Represents a configuration used to bootstrap a `Replica`.
pub struct ReplicaConfig<S, OP, ST, NT, PL> where
    S: Service + 'static,
    OP: StatefulOrderProtocol<S::Data, NT, PL> + 'static + PersistableOrderProtocol<OP::Serialization, OP::StateSerialization>,
    ST: StateTransferProtocol<S::Data, OP, NT, PL> + 'static + PersistableStateTransferProtocol,
    NT: Node<ServiceMsg<S::Data, OP::Serialization, ST::Serialization>>,
    PL: SMRPersistentLog<S::Data, OP::Serialization, OP::StateSerialization> {
    /// The application logic.
    pub service: S,

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
    /// The configuration for the State transfer protocol
    pub st_config: ST::Config,

    /// Check out the docs on `NodeConfig`.
    pub node: NT::Config,

    pub phantom: PhantomData<PL>
}