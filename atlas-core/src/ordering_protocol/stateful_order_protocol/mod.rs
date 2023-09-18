use atlas_common::error::Result;
use atlas_common::ordering::SeqNo;
use atlas_execution::serialize::ApplicationData;
use crate::ordering_protocol::{OrderingProtocol, OrderingProtocolArgs, PermissionedOrderingProtocol, SerProof, View};
use crate::ordering_protocol::networking::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};
use crate::persistent_log::StatefulOrderingProtocolLog;

pub type DecLog<D, OP, SOP> = <SOP as StatefulOrderProtocolMessage<D, OP>>::DecLog;

/// An order protocol that uses the log transfer protocol to manage its log
/// ATM this is limited to permissioned ordering protocols
pub trait StatefulOrderProtocol<D,  NT, PL>: OrderingProtocol<D, NT, PL> + PermissionedOrderingProtocol
    where D: ApplicationData + 'static {

    /// The serialization abstraction for ordering protocols with logs, so we can then send it across the network
    type StateSerialization: StatefulOrderProtocolMessage<D, Self::Serialization> + 'static;

    fn initialize_with_initial_state(config: Self::Config, args: OrderingProtocolArgs<D, NT, PL>,
                                     dec_log: DecLog<D, Self::Serialization, Self::StateSerialization>) -> Result<Self>
        where Self: Sized;

    /// Install a state received from other replicas in the system
    /// Should only alter the necessary things within its own state and
    /// then should return the state and a list of all requests that should
    /// then be executed by the application.
    fn install_state(&mut self, view_info: View<Self::PermissionedSerialization>,
                     dec_log: DecLog<D, Self::Serialization, Self::StateSerialization>) -> Result<Vec<D::Request>>
        where PL: StatefulOrderingProtocolLog<D, Self::Serialization, Self::StateSerialization, Self::PermissionedSerialization>;

    /// Snapshot the current log of the replica
    fn snapshot_log(&mut self) -> Result<(View<Self::PermissionedSerialization>, DecLog<D, Self::Serialization, Self::StateSerialization>)>
        where PL: StatefulOrderingProtocolLog<D, Self::Serialization, Self::StateSerialization, Self::PermissionedSerialization>;

    /// Get a reference to the current log of this ordering protocol
    fn current_log(&self) -> Result<&DecLog<D, Self::Serialization, Self::StateSerialization>>
        where PL: StatefulOrderingProtocolLog<D, Self::Serialization, Self::StateSerialization, Self::PermissionedSerialization>;

    /// Notify the consensus protocol that we have a checkpoint at a given sequence number,
    /// meaning it can now be garbage collected
    fn checkpointed(&mut self, seq: SeqNo) -> Result<()>
        where PL: StatefulOrderingProtocolLog<D, Self::Serialization, Self::StateSerialization, Self::PermissionedSerialization>;

    /// Get the proof for a given consensus instance
    fn get_proof(&self, seq: SeqNo) -> Result<Option<SerProof<D, Self::Serialization>>>;
}
