use std::marker::PhantomData;
use std::sync::Arc;

use atlas_common::error::*;
use atlas_common::phantom::FPhantom;
use atlas_communication::message::Header;
use atlas_communication::reconfiguration::NetworkInformationProvider;
use atlas_communication::serialization::{InternalMessageVerifier, Serializable};
use atlas_core::ordering_protocol::networking::serialize::OrderProtocolVerificationHelper;
use atlas_core::ordering_protocol::networking::serialize::{
    OrderingProtocolMessage, ViewTransferProtocolMessage,
};
use atlas_core::serialize::NoProtocol;
use atlas_logging_core::log_transfer::networking::serialize::LogTransferMessage;
use atlas_smr_application::serialize::ApplicationData;

use crate::message::{OrderableMessage, SystemMessage};
use crate::networking::signature_ver::SigVerifier;
use crate::state_transfer::networking::serialize::StateTransferMessage;
use crate::state_transfer::networking::signature_ver::StateTransferVerificationHelper;
use crate::SMRReq;

/// The type that encapsulates all the serializing, so we don't have to constantly use SystemMessage
pub struct Service<
    D: ApplicationData,
    P: OrderingProtocolMessage<SMRReq<D>>,
    L: LogTransferMessage<SMRReq<D>, P>,
    VT: ViewTransferProtocolMessage,
>(FPhantom<(D, P, L, VT)>);

pub type ServiceMessage<
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
> = <Service<D, P, L, VT> as Serializable>::Message;

impl<D, P, L, VT> Serializable for Service<D, P, L, VT>
where
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
{
    type Message = SystemMessage<D, P::ProtocolMessage, L::LogTransferMessage, VT::ProtocolMessage>;

    type Verifier = Self;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(
        builder: febft_capnp::messages_capnp::system::Builder,
        msg: &Self::Message,
    ) -> Result<()> {
        capnp::serialize_message::<D, P, S, L>(builder, msg)
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(
        reader: febft_capnp::messages_capnp::system::Reader,
    ) -> Result<Self::Message> {
        capnp::deserialize_message::<D, P, S, L>(reader)
    }
}

impl<D, P, L, VT> InternalMessageVerifier<ServiceMessage<D, P, L, VT>> for Service<D, P, L, VT>
where
    D: ApplicationData + 'static,
    P: OrderingProtocolMessage<SMRReq<D>> + 'static,
    L: LogTransferMessage<SMRReq<D>, P> + 'static,
    VT: ViewTransferProtocolMessage + 'static,
{
    fn verify_message<NI>(
        info_provider: &Arc<NI>,
        header: &Header,
        msg: &ServiceMessage<D, P, L, VT>,
    ) -> Result<()>
    where
        NI: NetworkInformationProvider + 'static,
    {
        match msg {
            SystemMessage::ProtocolMessage(protocol) => {
                P::internally_verify_message::<NI, SigVerifier<NI, D, P, L, VT>>(
                    info_provider,
                    header,
                    protocol.payload(),
                )?;

                Ok(())
            }
            SystemMessage::LogTransferMessage(log_transfer) => {
                L::verify_log_message::<NI, SigVerifier<NI, D, P, L, VT>>(
                    info_provider,
                    header,
                    log_transfer.payload(),
                )?;

                Ok(())
            }
            SystemMessage::ViewTransferMessage(view_transfer) => {
                VT::internally_verify_message::<NI>(
                    info_provider,
                    header,
                    view_transfer.payload(),
                )?;

                Ok(())
            }
            SystemMessage::ForwardedProtocolMessage(fwd_protocol) => {
                let message = fwd_protocol.message();

                P::internally_verify_message::<NI, SigVerifier<NI, D, P, L, VT>>(
                    info_provider,
                    message.header(),
                    message.message().payload(),
                )?;

                Ok(())
            }
            SystemMessage::ForwardedRequestMessage(fwd_requests) => {
                for stored_rq in fwd_requests.requests().iter() {
                    let header = stored_rq.header();
                    let message = stored_rq.message();

                    SigVerifier::<NI, D, P, L, VT>::verify_request_message(
                        info_provider,
                        header,
                        message.clone(),
                    )?;
                }

                Ok(())
            }
        }
    }
}

/// The state serialization type wrapper
pub struct StateSys<S>(PhantomData<fn() -> S>);

impl<S> Serializable for StateSys<S>
where
    S: StateTransferMessage,
{
    type Message = S::StateTransferMessage;
    type Verifier = Self;
}

impl<S> InternalMessageVerifier<S::StateTransferMessage> for StateSys<S>
where
    S: StateTransferMessage,
{
    fn verify_message<NI>(
        _info_provider: &Arc<NI>,
        _header: &Header,
        _message: &S::StateTransferMessage,
    ) -> Result<()>
    where
        NI: NetworkInformationProvider,
    {
        //TODO: actually verify
        Ok(())
    }
}

/// The type that encapsulates all the serializing of the SMR related messages
///
/// This will be utilized in the protocols as the Serializable type, in order to wrap
/// our applications data
pub struct SMRSysMsg<D>(PhantomData<fn() -> D>);

pub type SMRSysMessage<D: ApplicationData + 'static> = <SMRSysMsg<D> as Serializable>::Message;

impl<D> Serializable for SMRSysMsg<D>
where
    D: ApplicationData + 'static,
{
    type Message = OrderableMessage<D>;
    type Verifier = Self;
}

impl<D> InternalMessageVerifier<OrderableMessage<D>> for SMRSysMsg<D>
where
    D: ApplicationData,
{
    fn verify_message<NI>(
        _info_provider: &Arc<NI>,
        _header: &Header,
        _message: &OrderableMessage<D>,
    ) -> Result<()>
    where
        NI: NetworkInformationProvider,
    {
        // Client requests don't need to be internally verified, as they only contain application data.
        // The signature verification is done on the network level
        Ok(())
    }
}

impl StateTransferMessage for NoProtocol {
    type StateTransferMessage = ();

    fn verify_state_message<NI, SVH>(
        _network_info: &Arc<NI>,
        _header: &Header,
        message: Self::StateTransferMessage,
    ) -> Result<Self::StateTransferMessage>
    where
        NI: NetworkInformationProvider,
        SVH: StateTransferVerificationHelper,
    {
        //TODO: actually verify
        Ok(message)
    }

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(
        _: febft_capnp::cst_messages_capnp::cst_message::Builder,
        msg: &Self::StateTransferMessage,
    ) -> Result<()> {
        unimplemented!()
    }

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(
        _: febft_capnp::cst_messages_capnp::cst_message::Reader,
    ) -> Result<Self::StateTransferMessage> {
        unimplemented!()
    }
}
