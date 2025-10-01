use std::fmt::{Debug, Formatter};
use std::ops::Deref;

use atlas_common::ordering::{Orderable, SeqNo};
use serde::{Deserialize, Serialize};

use atlas_communication::message::StoredMessage;
use atlas_core::messages::{
    ForwardedProtocolMessage, ForwardedRequestsMessage, Protocol, SessionBased, VTMessage,
};
use atlas_logging_core::log_transfer::networking::LogTransfer;
use atlas_smr_application::serialize::ApplicationData;

use crate::exec::RequestType;
use crate::{SMRReply, SMRReq};

///A reply to an unordered request
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum OrderableMessage<D: ApplicationData> {
    ///An ordered request
    /// You can check what these bounds mean here: https://serde.rs/attr-bound.html
    #[serde(bound(
        deserialize = "D::Request: Deserialize<'de>",
        serialize = "D::Request: Serialize"
    ))]
    OrderedRequest(SMRReq<D>),
    ///An unordered request
    /// You can check what these bounds mean here: https://serde.rs/attr-bound.html
    #[serde(bound(
        deserialize = "D::Request: Deserialize<'de>",
        serialize = "D::Request: Serialize"
    ))]
    UnorderedRequest(SMRReq<D>),
    ///A reply to an ordered request
    /// You can check what these bounds mean here: https://serde.rs/attr-bound.html
    #[serde(bound(
        deserialize = "D::Reply: Deserialize<'de>",
        serialize = "D::Reply: Serialize"
    ))]
    OrderedReply(SMRReply<D>),
    ///A reply to an unordered request
    /// You can check what these bounds mean here: https://serde.rs/attr-bound.html
    #[serde(bound(
        deserialize = "D::Reply: Deserialize<'de>",
        serialize = "D::Reply: Serialize"
    ))]
    UnorderedReply(SMRReply<D>),
}

/// The message enum that encapsulates all messages that are sent in the SMR protocol
/// These messages are only the ones that are going to be sent between participating replicas
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub enum SystemMessage<D: ApplicationData, P, LT, VT> {
    ///Requests forwarded from other peers
    /// You can check what these bounds mean here: https://serde.rs/attr-bound.html
    #[serde(bound(
        deserialize = "D::Request: Deserialize<'de>",
        serialize = "D::Request: Serialize"
    ))]
    ForwardedRequestMessage(ForwardedRequestsMessage<SMRReq<D>>),
    ///A message related to the protocol
    ProtocolMessage(Protocol<P>),
    ///A protocol message that has been forwarded by another peer
    ForwardedProtocolMessage(ForwardedProtocolMessage<P>),
    ///A Log transfer protocol message
    LogTransferMessage(LogTransfer<LT>),
    /// View Transfer protocol message
    ViewTransferMessage(VTMessage<VT>),
}

impl<D, P, LT, VT> SystemMessage<D, P, LT, VT>
where
    D: ApplicationData,
{
    pub fn from_protocol_message(msg: P) -> Self {
        SystemMessage::ProtocolMessage(Protocol::new(msg))
    }

    pub fn from_log_transfer_message(msg: LT) -> Self {
        SystemMessage::LogTransferMessage(LogTransfer::new(msg))
    }

    pub fn from_view_transfer_message(msg: VT) -> Self {
        SystemMessage::ViewTransferMessage(VTMessage::new(msg))
    }

    pub fn from_fwd_protocol_message(msg: StoredMessage<Protocol<P>>) -> Self {
        SystemMessage::ForwardedProtocolMessage(ForwardedProtocolMessage::new(msg))
    }

    pub fn into_protocol_message(self) -> P {
        match self {
            SystemMessage::ProtocolMessage(prot) => prot.into_inner(),
            _ => {
                unreachable!()
            }
        }
    }

    pub fn into_log_transfer_message(self) -> LT {
        match self {
            SystemMessage::LogTransferMessage(l) => l.into_inner(),
            _ => {
                unreachable!()
            }
        }
    }

    pub fn into_view_transfer_message(self) -> VT {
        match self {
            SystemMessage::ViewTransferMessage(vt) => vt.into_inner(),
            _ => {
                unreachable!()
            }
        }
    }
}

impl<D, P, LT, VT> Clone for SystemMessage<D, P, LT, VT>
where
    D: ApplicationData,
    P: Clone,
    LT: Clone,
    VT: Clone,
{
    fn clone(&self) -> Self {
        match self {
            SystemMessage::ForwardedRequestMessage(fwd_req) => {
                SystemMessage::ForwardedRequestMessage(fwd_req.clone())
            }
            SystemMessage::ProtocolMessage(prot) => SystemMessage::ProtocolMessage(prot.clone()),
            SystemMessage::ForwardedProtocolMessage(prot) => {
                SystemMessage::ForwardedProtocolMessage(prot.clone())
            }
            SystemMessage::LogTransferMessage(log_transfer) => {
                SystemMessage::LogTransferMessage(log_transfer.clone())
            }
            SystemMessage::ViewTransferMessage(view_transfer) => {
                SystemMessage::ViewTransferMessage(view_transfer.clone())
            }
        }
    }
}

impl<D> Clone for OrderableMessage<D>
where
    D: ApplicationData,
{
    fn clone(&self) -> Self {
        match self {
            OrderableMessage::OrderedRequest(req) => OrderableMessage::OrderedRequest(req.clone()),
            OrderableMessage::UnorderedRequest(req) => {
                OrderableMessage::UnorderedRequest(req.clone())
            }
            OrderableMessage::OrderedReply(rep) => OrderableMessage::OrderedReply(rep.clone()),
            OrderableMessage::UnorderedReply(rep) => OrderableMessage::UnorderedReply(rep.clone()),
        }
    }
}

impl<D> SessionBased for OrderableMessage<D>
where
    D: ApplicationData,
{
    fn session_number(&self) -> SeqNo {
        match self {
            OrderableMessage::OrderedRequest(rq) | OrderableMessage::UnorderedRequest(rq) => {
                rq.session_number()
            }
            OrderableMessage::OrderedReply(rp) | OrderableMessage::UnorderedReply(rp) => {
                rp.session_number()
            }
        }
    }
}

impl<D, P, LT, VT> Debug for SystemMessage<D, P, LT, VT>
where
    D: ApplicationData,
    P: Clone,
    LT: Clone,
    VT: Clone,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemMessage::ForwardedRequestMessage(_) => {
                write!(f, "Forwarded Request")
            }
            SystemMessage::ProtocolMessage(_) => {
                write!(f, "Protocol Message")
            }
            SystemMessage::ForwardedProtocolMessage(_) => {
                write!(f, "Forwarded Protocol Message")
            }
            SystemMessage::LogTransferMessage(_) => {
                write!(f, "Log transfer message")
            }
            SystemMessage::ViewTransferMessage(_) => {
                write!(f, "View Transfer message")
            }
        }
    }
}

impl<D: ApplicationData> OrderableMessage<D> {
    //Into SMR Request
    pub fn into_smr_request(self) -> SMRReq<D> {
        match self {
            OrderableMessage::OrderedRequest(req) => req,
            OrderableMessage::UnorderedRequest(req) => req,
            _ => {
                unreachable!()
            }
        }
    }

    //Into SMR Reply
    pub fn into_smr_reply(self) -> SMRReply<D> {
        match self {
            OrderableMessage::OrderedReply(rep) => rep,
            OrderableMessage::UnorderedReply(rep) => rep,
            _ => {
                unreachable!()
            }
        }
    }
}

impl<D: ApplicationData> From<(RequestType, SMRReq<D>)> for OrderableMessage<D> {
    fn from(value: (RequestType, SMRReq<D>)) -> Self {
        match value.0 {
            RequestType::Ordered => OrderableMessage::OrderedRequest(value.1),
            RequestType::Unordered => OrderableMessage::UnorderedRequest(value.1),
        }
    }
}

impl<D: ApplicationData> Orderable for OrderableMessage<D> {
    fn sequence_number(&self) -> SeqNo {
        match self {
            OrderableMessage::OrderedRequest(req) => req.sequence_number(),
            OrderableMessage::UnorderedRequest(req) => req.sequence_number(),
            OrderableMessage::OrderedReply(rep) => rep.sequence_number(),
            OrderableMessage::UnorderedReply(rep) => rep.sequence_number(),
        }
    }
}

///
/// State transfer messages
///
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub struct StateTransfer<P> {
    payload: P,
}

impl<P> StateTransfer<P> {
    pub fn new(payload: P) -> Self {
        Self { payload }
    }

    pub fn payload(&self) -> &P {
        &self.payload
    }

    pub fn into_inner(self) -> P {
        self.payload
    }
}

impl<P> Deref for StateTransfer<P> {
    type Target = P;

    fn deref(&self) -> &Self::Target {
        &self.payload
    }
}
