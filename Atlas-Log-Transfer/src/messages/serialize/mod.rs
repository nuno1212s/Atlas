use crate::messages::{LTMessage, LogTransferMessageKind};
use atlas_common::ordering::Orderable;
use atlas_common::phantom::FPhantom;
use atlas_common::serialization_helper::SerMsg;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration::NetworkInformationProvider;
use atlas_core::ordering_protocol::loggable::message::PersistentOrderProtocolTypes;
use atlas_core::ordering_protocol::networking::serialize::OrderingProtocolMessage;
use atlas_logging_core::decision_log::serialize::DecisionLogMessage;
use atlas_logging_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_logging_core::log_transfer::networking::signature_ver::LogTransferVerificationHelper;
use std::sync::Arc;

pub struct LTMsg<
    RQ: SerMsg,
    OP: OrderingProtocolMessage<RQ>,
    POPT: PersistentOrderProtocolTypes<RQ, OP>,
    LS: DecisionLogMessage<RQ, OP, POPT>,
>(FPhantom<(RQ, OP, POPT, LS)>);

impl<
        RQ: SerMsg,
        OP: OrderingProtocolMessage<RQ>,
        POPT: PersistentOrderProtocolTypes<RQ, OP>,
        LS: DecisionLogMessage<RQ, OP, POPT>,
    > LogTransferMessage<RQ, OP> for LTMsg<RQ, OP, POPT, LS>
{
    type LogTransferMessage = LTMessage<POPT::Proof, LS::DecLog>;

    fn verify_log_message<NI, LVH>(
        network_info: &Arc<NI>,
        _header: &Header,
        message: &Self::LogTransferMessage,
    ) -> atlas_common::error::Result<()>
    where
        NI: NetworkInformationProvider,
        LVH: LogTransferVerificationHelper<RQ, OP, NI>,
    {
        let _seq = message.sequence_number();

        match message.kind() {
            LogTransferMessageKind::RequestLogState => Ok(()),
            LogTransferMessageKind::ReplyLogState(opt) => {
                if let Some((_first_seq, (_last_seq, proof))) = opt {
                    let _proof = POPT::verify_proof::<NI, LVH>(network_info, proof.clone())?;

                    Ok(())
                } else {
                    Ok(())
                }
            }
            LogTransferMessageKind::RequestProofs(_seqs) => Ok(()),
            LogTransferMessageKind::ReplyLogParts(proofs) => {
                for (_seq, proof) in proofs {
                    let _proof = POPT::verify_proof::<NI, LVH>(network_info, proof.clone())?;
                }

                Ok(())
            }
            LogTransferMessageKind::RequestLog => Ok(()),
            LogTransferMessageKind::ReplyLog(dec_log) => {
                let _dec_log = LS::verify_decision_log::<NI, LVH>(network_info, dec_log.clone())?;

                Ok(())
            }
        }
    }

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(
        builder: atlas_capnp::lt_messages_capnp::lt_message::Builder,
        msg: &Self::LogTransferMessage,
    ) -> atlas_common::error::Result<()> {
        todo!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(
        reader: atlas_capnp::lt_messages_capnp::lt_message::Reader,
    ) -> atlas_common::error::Result<Self::LogTransferMessage> {
        todo!()
    }
}
