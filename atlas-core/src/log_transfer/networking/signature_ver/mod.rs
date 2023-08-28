use atlas_communication::FullNetworkNode;
use atlas_communication::message_signing::NetworkMessageSignatureVerifier;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;
use atlas_communication::serialize::Serializable;
use atlas_execution::serialize::ApplicationData;
use crate::log_transfer::networking::serialize::LogTransferMessage;
use crate::messages::signature_ver::SigVerifier;
use crate::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use crate::ordering_protocol::networking::signature_ver::OrderProtocolSignatureVerificationHelper;
use crate::serialize::Service;
use crate::state_transfer::networking::serialize::StateTransferMessage;

pub trait LogTransferVerificationHelper<D, OP, NI>: OrderProtocolSignatureVerificationHelper<D, OP, NI>
    where D: ApplicationData, OP: OrderingProtocolMessage<D>, NI: NetworkInformationProvider {}

impl<SV, NI, D, OP, ST, LT> LogTransferVerificationHelper<D, OP, NI> for SigVerifier<SV, NI, D, OP, ST, LT>
    where D: ApplicationData + 'static,
          OP: OrderingProtocolMessage<D> + 'static,
          ST: StateTransferMessage + 'static,
          LT: LogTransferMessage<D, OP> + 'static,
          NI: NetworkInformationProvider + 'static,
          SV: NetworkMessageSignatureVerifier<Service<D, OP, ST, LT>, NI> {}