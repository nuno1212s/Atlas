use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use std::ops::{Deref, DerefMut};

#[derive(Clone, Debug)]
pub enum UpdateInfo {
    SessionBased {
        from: NodeId,
        session_number: SeqNo,
        sequence_number: SeqNo,
    },
}

impl UpdateInfo {
    pub fn new_session_based(from: NodeId, session_number: SeqNo, sequence_number: SeqNo) -> Self {
        UpdateInfo::SessionBased {
            from,
            session_number,
            sequence_number,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Update<O> {
    info: UpdateInfo,
    operation: O,
}

impl<O> Update<O> {
    pub fn new(info: UpdateInfo, operation: O) -> Self {
        Self { info, operation }
    }

    pub fn info(&self) -> &UpdateInfo {
        &self.info
    }

    pub fn operation(&self) -> &O {
        &self.operation
    }

    pub fn into_inner(self) -> (UpdateInfo, O) {
        (self.info, self.operation)
    }
}

pub struct UpdateReply<R> {
    info: UpdateInfo,
    reply: R,
}

impl<R> UpdateReply<R> {
    pub fn new(info: UpdateInfo, reply: R) -> Self {
        Self { info, reply }
    }

    pub fn info(&self) -> &UpdateInfo {
        &self.info
    }

    pub fn reply(&self) -> &R {
        &self.reply
    }

    pub fn into_inner(self) -> (UpdateInfo, R) {
        (self.info, self.reply)
    }
}

#[derive(Clone)]
pub struct UpdateBatch<O> {
    seq_no: SeqNo,
    updates: Vec<Update<O>>,
}

impl<O> UpdateBatch<O> {
    /// Returns a new, empty batch of requests.
    pub fn new(seq_no: SeqNo) -> Self {
        Self {
            seq_no,
            updates: Vec::new(),
        }
    }

    pub fn new_with_cap(seq_no: SeqNo, cap: usize) -> Self {
        Self {
            seq_no,
            updates: Vec::with_capacity(cap),
        }
    }
    pub fn seq_no(&self) -> SeqNo {
        self.seq_no
    }

    /// Returns the length of the batch.
    pub fn len(&self) -> usize {
        self.updates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }

    pub fn into_inner(self) -> (SeqNo, Vec<Update<O>>) {
        (self.seq_no, self.updates)
    }
}

impl<O> IncrementableUpdateBatch<O> for UpdateBatch<O> {
    fn add(&mut self, update_info: UpdateInfo, operation: O) {
        let update = Update::new(update_info, operation);

        self.updates.push(update);
    }
}

impl<O> Orderable for UpdateBatch<O> {
    fn sequence_number(&self) -> SeqNo {
        self.seq_no
    }
}

pub struct ReplyBatch<R> {
    replies: Vec<UpdateReply<R>>,
}

impl<R> ReplyBatch<R> {
    pub fn new_with_cap(cap: usize) -> Self {
        Self {
            replies: Vec::with_capacity(cap),
        }
    }

    /// Returns the length of the batch.
    pub fn len(&self) -> usize {
        self.replies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.replies.is_empty()
    }

    pub fn into_inner(self) -> Vec<UpdateReply<R>> {
        self.replies
    }
}

impl<R> Deref for ReplyBatch<R> {
    type Target = Vec<UpdateReply<R>>;

    fn deref(&self) -> &Self::Target {
        &self.replies
    }
}

impl<R> DerefMut for ReplyBatch<R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.replies
    }
}

impl<R> IncrementableUpdateBatch<R> for ReplyBatch<R> {
    fn add(&mut self, update_info: UpdateInfo, reply: R) {
        let update_reply = UpdateReply::new(update_info, reply);

        self.replies.push(update_reply);
    }
}

impl<R> Default for ReplyBatch<R> {
    fn default() -> Self {
        Self {
            replies: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct UnorderedUpdateBatch<O> {
    requests: Vec<Update<O>>,
}

impl<O> UnorderedUpdateBatch<O> {
    /// Returns a new, empty batch of unordered requests.
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
        }
    }

    pub fn new_with_cap(cap: usize) -> Self {
        Self {
            requests: Vec::with_capacity(cap),
        }
    }

    /// Returns the length of the batch.
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub fn into_inner(self) -> Vec<Update<O>> {
        self.requests
    }
}

impl<O> IncrementableUpdateBatch<O> for UnorderedUpdateBatch<O> {
    fn add(&mut self, update_info: UpdateInfo, operation: O) {
        let update = Update::new(update_info, operation);

        self.requests.push(update);
    }
}

impl<O> Deref for UnorderedUpdateBatch<O> {
    type Target = Vec<Update<O>>;

    fn deref(&self) -> &Self::Target {
        &self.requests
    }
}

pub trait IncrementableUpdateBatch<O> {
    fn add(&mut self, update_info: UpdateInfo, operation: O);
}
