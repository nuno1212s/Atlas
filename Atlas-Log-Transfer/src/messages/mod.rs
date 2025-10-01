pub mod serialize;

#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use atlas_common::ordering::{Orderable, SeqNo};

#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub struct LTMessage<P, DL> {
    // NOTE: not the same sequence number used in the
    // consensus layer to order client requests!
    seq: SeqNo,
    kind: LogTransferMessageKind<P, DL>,
}

#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub enum LogTransferMessageKind<P, DL> {
    RequestLogState,
    ReplyLogState(Option<(SeqNo, (SeqNo, P))>),
    RequestProofs(Vec<SeqNo>),
    ReplyLogParts(Vec<(SeqNo, P)>),
    RequestLog,
    ReplyLog(DL),
}

impl<P, DL> LTMessage<P, DL> {
    /// Creates a new `CstMessage` with sequence number `seq`,
    /// and of the kind `kind`.
    pub fn new(seq: SeqNo, kind: LogTransferMessageKind<P, DL>) -> Self {
        Self { seq, kind }
    }

    pub fn kind(&self) -> &LogTransferMessageKind<P, DL> {
        &self.kind
    }

    pub fn into_kind(self) -> LogTransferMessageKind<P, DL> {
        self.kind
    }
}

impl<P, DL> Orderable for LTMessage<P, DL> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

///Debug for LogTransferMessage
impl<P, DL> Debug for LTMessage<P, DL> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            LogTransferMessageKind::RequestLogState => {
                write!(f, "Request log state")
            }
            LogTransferMessageKind::ReplyLogState(opt) => {
                write!(
                    f,
                    "Reply log state {:?}",
                    opt.as_ref()
                        .map(|(seq, (last, _))| (*seq, *last))
                        .unwrap_or((SeqNo::ZERO, SeqNo::ZERO))
                )
            }
            LogTransferMessageKind::RequestProofs(_) => {
                write!(f, "Request log proofs")
            }
            LogTransferMessageKind::ReplyLogParts(_) => {
                write!(f, "Reply log parts")
            }
            LogTransferMessageKind::RequestLog => {
                write!(f, "Request log")
            }
            LogTransferMessageKind::ReplyLog(_) => {
                write!(f, "Reply log")
            }
        }
    }
}
