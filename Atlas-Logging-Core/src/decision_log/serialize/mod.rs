use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::reconfiguration::NetworkInformationProvider;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::networking::serialize::{
    OrderProtocolVerificationHelper, OrderingProtocolMessage,
};
use std::sync::Arc;

pub trait OrderProtocolLog: Orderable {
    // At the moment I only need orderable, but I might need more in the future
    fn first_seq(&self) -> Option<SeqNo>;
}

/// A trait defining what we need in order to verify parts of the decision log
pub trait OrderProtocolLogPart: Orderable {
    // We only need to add the first sequence number, since we already know the last
    // From the orderable implementation
    fn first_seq(&self) -> Option<SeqNo>;
}

pub trait DecisionLogMessage<RQ, OPM, POP>: Send + Sync + 'static {
    /// A metadata type to allow for decision logs to include some
    /// more specific information into their decision log, apart from
    /// the list of proofs
    type DecLogMetadata: SerMsg;

    /// A type that defines the log of decisions made since the last garbage collection
    /// (In the case of BFT SMR the log is GCed after a checkpoint of the application)
    type DecLog: OrderProtocolLog + SerMsg;

    /// A type that defines the log of decisions made since the last garbage collection
    /// (In the case of BFT SMR the log is GCed after a checkpoint of the application)
    type DecLogPart: OrderProtocolLogPart + SerMsg;

    fn verify_decision_log<NI, OPVH>(
        network_info: &Arc<NI>,
        dec_log: Self::DecLog,
    ) -> atlas_common::error::Result<Self::DecLog>
    where
        NI: NetworkInformationProvider,
        OPM: OrderingProtocolMessage<RQ>,
        POP: PersistentOrderProtocolTypes<RQ, OPM>,
        OPVH: OrderProtocolVerificationHelper<RQ, OPM, NI>;
}
