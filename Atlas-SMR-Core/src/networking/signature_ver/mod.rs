use std::marker::PhantomData;
use std::sync::Arc;

use atlas_communication::message::Header;
use atlas_communication::reconfiguration::NetworkInformationProvider;
use atlas_core::ordering_protocol::networking::serialize::{
    OrderProtocolVerificationHelper, OrderingProtocolMessage, ViewTransferProtocolMessage,
};
use atlas_logging_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_logging_core::log_transfer::networking::signature_ver::LogTransferVerificationHelper;
use atlas_smr_application::serialize::ApplicationData;

use crate::message::{OrderableMessage, SystemMessage};
use crate::serialize::{SMRSysMsg, Service};
use crate::SMRReq;

type CPhantom<T> = PhantomData<fn() -> T>;

pub struct SigVerifier<NI, D, OP, LT, VT>(CPhantom<(NI, D, OP, LT, VT)>);

impl<NI, D, P, L, VT> OrderProtocolVerificationHelper<SMRReq<D>, P, NI>
    for SigVerifier<NI, D, P, L, VT>
where
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    NI: 'static,
{
    fn verify_request_message(
        network_info: &Arc<NI>,
        header: &Header,
        request: SMRReq<D>,
    ) -> atlas_common::error::Result<SMRReq<D>>
    where
        NI: NetworkInformationProvider + 'static,
    {
        let message = OrderableMessage::<D>::OrderedRequest(request);

        atlas_communication::message_signing::verify_message_validity::<SMRSysMsg<D>>(
            &**network_info,
            header,
            &message,
        )?;

        if let OrderableMessage::OrderedRequest(r) = message {
            Ok(r)
        } else {
            unreachable!()
        }
    }

    fn verify_protocol_message(
        network_info: &Arc<NI>,
        header: &Header,
        message: P::ProtocolMessage,
    ) -> atlas_common::error::Result<P::ProtocolMessage>
    where
        NI: NetworkInformationProvider + 'static,
    {
        let message = SystemMessage::<
            D,
            P::ProtocolMessage,
            L::LogTransferMessage,
            VT::ProtocolMessage,
        >::from_protocol_message(message);

        atlas_communication::message_signing::verify_message_validity::<Service<D, P, L, VT>>(
            &**network_info,
            header,
            &message,
        )?;

        if let SystemMessage::ProtocolMessage(r) = message {
            Ok(r.into_inner())
        } else {
            unreachable!()
        }
    }
}

impl<NI, D, OP, LT, VT> LogTransferVerificationHelper<SMRReq<D>, OP, NI>
    for SigVerifier<NI, D, OP, LT, VT>
where
    D: ApplicationData + 'static,
    OP: OrderingProtocolMessage<SMRReq<D>> + 'static,
    LT: LogTransferMessage<SMRReq<D>, OP> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
    NI: NetworkInformationProvider + 'static,
{
}
