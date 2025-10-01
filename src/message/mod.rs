use atlas_common::ordering::{Orderable, SeqNo};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

pub mod serialize;

/// The view protocol transfer message
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug)]
pub struct ViewTransferMessage<V> {
    sequence: SeqNo,
    view_transfer_message_kind: ViewTransferMessageKind<V>,
}

/// The types of view transfer messages
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
#[derive(Clone, Debug)]
pub enum ViewTransferMessageKind<V> {
    RequestView,
    ViewResponse(V),
}

impl<V> ViewTransferMessage<V> {
    pub fn new(seq: SeqNo, message_kind: ViewTransferMessageKind<V>) -> Self {
        Self {
            sequence: seq,
            view_transfer_message_kind: message_kind,
        }
    }

    pub fn kind(&self) -> &ViewTransferMessageKind<V> {
        &self.view_transfer_message_kind
    }

    pub fn into_kind(self) -> ViewTransferMessageKind<V> {
        self.view_transfer_message_kind
    }
}

impl<V> Orderable for ViewTransferMessage<V> {
    fn sequence_number(&self) -> SeqNo {
        self.sequence
    }
}
