pub mod serialize;

use std::fmt::Debug;
#[cfg(feature = "serialize_serde")]
use serde::{Serialize, Deserialize};

use atlas_common::ordering::{Orderable, SeqNo};

#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub struct LTMessage<V, P, DL> {
    // NOTE: not the same sequence number used in the
    // consensus layer to order client requests!
    seq: SeqNo,
    kind: LogTransferMessageKind<V, P, DL>,
}

#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone)]
pub enum LogTransferMessageKind<V, P, DL> {
    RequestLogState,
    ReplyLogState(V, Option<(SeqNo, (SeqNo, P))>),
    RequestProofs(Vec<SeqNo>),
    ReplyLogParts(V, Vec<(SeqNo, P)>),
    RequestLog,
    ReplyLog(V, DL)
}

impl<V, P, DL> LTMessage<V, P, DL> {
    /// Creates a new `CstMessage` with sequence number `seq`,
    /// and of the kind `kind`.
    pub fn new(seq: SeqNo, kind: LogTransferMessageKind<V, P, DL>) -> Self {
        Self { seq, kind }
    }

    pub fn kind(&self) -> &LogTransferMessageKind<V, P, DL> {
        &self.kind
    }
    
    pub fn into_kind(self) -> LogTransferMessageKind<V, P, DL> {
        self.kind
    }

}

impl<V, P, DL> Orderable for LTMessage<V, P, DL> {
    fn sequence_number(&self) -> SeqNo {
        self.seq
    }
}

///Debug for LogTransferMessage
impl<V, P, DL> Debug for LTMessage<V, P, DL> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            LogTransferMessageKind::RequestLogState => {
                write!(f, "Request log state")
            }
            LogTransferMessageKind::ReplyLogState(_ ,opt ) => {
                write!(f, "Reply log state {:?}", opt.as_ref().map(|(seq, (last, _))| (*seq, *last)).unwrap_or((SeqNo::ZERO, SeqNo::ZERO)))
            }
            LogTransferMessageKind::RequestProofs(_) => {
                write!(f, "Request log proofs")
            }
            LogTransferMessageKind::ReplyLogParts(_, _) => {
                write!(f, "Reply log parts")
            }
            LogTransferMessageKind::RequestLog => {
                write!(f, "Request log")
            }
            LogTransferMessageKind::ReplyLog(_, _) => {
                write!(f, "Reply log")
            }
        }
    }
}

