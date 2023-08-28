use std::marker::PhantomData;
use std::sync::Arc;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration_node::NetworkInformationProvider;

use atlas_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_core::log_transfer::networking::signature_ver::LogTransferVerificationHelper;
use atlas_core::ordering_protocol::networking::serialize::{OrderingProtocolMessage, StatefulOrderProtocolMessage};
use atlas_execution::serialize::ApplicationData;

use crate::messages::{LogTransferMessageKind, LTMessage};

pub struct LTMsg<D: ApplicationData, OP: OrderingProtocolMessage<D>, SOP: StatefulOrderProtocolMessage<D, OP>>(PhantomData<(D, OP, SOP)>);

impl<D: ApplicationData, OP: OrderingProtocolMessage<D>, SOP: StatefulOrderProtocolMessage<D, OP>> LogTransferMessage<D, OP> for LTMsg<D, OP, SOP> {
    type LogTransferMessage = LTMessage<OP::ViewInfo, OP::Proof, SOP::DecLog>;

    fn verify_log_message<NI, LVH>(network_info: &Arc<NI>, header: &Header, message: Self::LogTransferMessage) -> atlas_common::error::Result<(bool, Self::LogTransferMessage)>
        where NI: NetworkInformationProvider, LVH: LogTransferVerificationHelper<D, OP, NI>, {
        match message.kind() {
            LogTransferMessageKind::RequestLogState => {
                Ok((true, message))
            }
            LogTransferMessageKind::ReplyLogState(view, opt) => {
                if let Some((first_seq, (last_seq, proof))) = opt {
                    let (result, proof) = OP::verify_proof::<NI, LVH>(network_info, proof.clone())?;

                    Ok((result, message))
                } else {
                    Ok((true, message))
                }
            }
            LogTransferMessageKind::RequestProofs(_) => {
                Ok((true, message))
            }
            LogTransferMessageKind::ReplyLogParts(vview, proofs) => {
                for (seq, proof) in proofs {
                    let (result, proof) = OP::verify_proof::<NI, LVH>(network_info, proof.clone())?;

                    if !result {
                        return Ok((false, message));
                    }
                }

                Ok((true, message))
            }
            LogTransferMessageKind::RequestLog => {
                Ok((true, message))
            }
            LogTransferMessageKind::ReplyLog(view_info, dec_log) => {
                let (result, dec_log) = SOP::verify_decision_log::<NI, LVH>(network_info, dec_log.clone())?;

                Ok((result, message))
            }
        }
    }

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: atlas_capnp::lt_messages_capnp::lt_message::Builder, msg: &Self::LogTransferMessage) -> atlas_common::error::Result<()> {
        todo!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: atlas_capnp::lt_messages_capnp::lt_message::Reader) -> atlas_common::error::Result<Self::LogTransferMessage> {
        todo!()
    }
}