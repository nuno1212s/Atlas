use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_execution::serialize::ApplicationData;
use crate::ordering_protocol::networking::signature_ver::OrderProtocolSignatureVerificationHelper;
use crate::serialize::OrderingProtocolMessage;

pub trait LogTransferVerificationHelper<D, OP, NI>: OrderProtocolSignatureVerificationHelper<D, OP, NI>
    where D: ApplicationData, OP: OrderingProtocolMessage, NI: NetworkInformationProvider {}