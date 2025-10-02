use atlas_communication::reconfiguration::NetworkInformationProvider;

use atlas_core::ordering_protocol::networking::serialize::{
    OrderProtocolVerificationHelper, OrderingProtocolMessage,
};

pub trait LogTransferVerificationHelper<RQ, OP, NI>:
    OrderProtocolVerificationHelper<RQ, OP, NI>
where
    OP: OrderingProtocolMessage<RQ>,
    NI: NetworkInformationProvider,
{
}
