use crate::serialize::ApplicationData;
use atlas_common::error::*;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_metrics::benchmarks::BatchMeta;
use std::ops::{Deref, DerefMut};

/// Request type of the `Service`.
pub type Request<A, S> = <<A as Application<S>>::AppData as ApplicationData>::Request;

/// Reply type of the `Service`.
pub type Reply<A, S> = <<A as Application<S>>::AppData as ApplicationData>::Reply;

pub type AppData<A, S> = <A as Application<S>>::AppData;

/// An application for a state machine replication protocol.
/// Applications must be [Sync] and [Send] as they can be called
/// from multiple threads. The concurrency control should be done
/// by the State, never the actual application.
///
/// We only pass the self reference for convenience, as the application
/// in theory would only require the state and the request.
pub trait Application<S>: Send + Sync {
    type AppData: ApplicationData + 'static;

    /// Returns the initial state of the application.
    fn initial_state() -> Result<S>;

    /// Process an unordered client request, and produce a matching reply
    /// Cannot alter the application state
    fn unordered_execution(&self, state: &S, request: Request<Self, S>) -> Reply<Self, S>;

    /// Much like [`unordered_execution()`], but processes a batch of requests.
    ///
    /// If [`unordered_batched_execution()`] is defined by the user, then [`unordered_execution()`] may
    /// simply be defined as such:
    ///
    /// ```rust
    /// fn unordered_execution(
    /// state: &S,
    /// request: Request<Self, S>) -> Reply<Self, S> {
    ///     unimplemented!()
    /// }
    /// ```
    fn unordered_batched_execution(
        &self,
        state: &S,
        requests: UnorderedBatch<Request<Self, S>>,
    ) -> BatchReplies<Reply<Self, S>> {
        let mut reply_batch = BatchReplies::with_capacity(requests.len());

        for unordered_req in requests.into_inner() {
            let (peer_id, sess, opid, req) = unordered_req.into_inner();
            let reply = self.unordered_execution(state, req);
            reply_batch.add(peer_id, sess, opid, reply);
        }

        reply_batch
    }

    /// Process a user request, producing a matching reply,
    /// meanwhile updating the application state.
    fn update(&self, state: &mut S, request: Request<Self, S>) -> Reply<Self, S>;

    /// Much like `update()`, but processes a batch of requests.
    ///
    /// If `update_batch()` is defined by the user, then `update()` may
    /// simply be defined as such:
    ///
    /// ```rust
    /// fn update(
    ///     state: &mut State<Self>,
    ///     request: Request<Self>,
    /// ) -> Reply<Self> {
    ///     unimplemented!()
    /// }
    /// ```
    fn update_batch(
        &self,
        state: &mut S,
        batch: UpdateBatch<Request<Self, S>>,
    ) -> BatchReplies<Reply<Self, S>> {
        let mut reply_batch = BatchReplies::with_capacity(batch.len());

        for update in batch.into_inner() {
            let (peer_id, sess, opid, req) = update.into_inner();
            let reply = self.update(state, req);
            reply_batch.add(peer_id, sess, opid, reply);
        }

        reply_batch
    }
}

/// Represents a single client update request, to be executed.
#[derive(Clone)]
pub struct Update<O> {
    from: NodeId,
    session_id: SeqNo,
    operation_id: SeqNo,
    operation: O,
}

/// Represents a single client update reply.
#[derive(Clone)]
pub struct UpdateReply<P> {
    to: NodeId,
    session_id: SeqNo,
    operation_id: SeqNo,
    payload: P,
}

/// Storage for a batch of client update requests to be executed.
#[derive(Clone)]
pub struct UnorderedBatch<O> {
    inner: Vec<Update<O>>,
}

/// Storage for a batch of client update requests to be executed.
#[derive(Clone)]
pub struct UpdateBatch<O> {
    seq_no: SeqNo,
    inner: Vec<Update<O>>,
    meta: Option<BatchMeta>,
}

/// Storage for a batch of client update replies.
#[derive(Clone)]
pub struct BatchReplies<P> {
    inner: Vec<UpdateReply<P>>,
}

impl<O> UpdateBatch<O> {
    /// Returns a new, empty batch of requests.
    pub fn new(seq_no: SeqNo) -> Self {
        Self {
            seq_no,
            inner: Vec::new(),
            meta: None,
        }
    }

    pub fn new_with_cap(seq_no: SeqNo, capacity: usize) -> Self {
        Self {
            seq_no,
            inner: Vec::with_capacity(capacity),
            meta: None,
        }
    }

    /// Adds a new update request to the batch.
    pub fn add(&mut self, from: NodeId, session_id: SeqNo, operation_id: SeqNo, operation: O) {
        self.inner.push(Update {
            from,
            session_id,
            operation_id,
            operation,
        });
    }

    /// Returns the inner storage.
    pub fn into_inner(self) -> Vec<Update<O>> {
        self.inner
    }

    /// Returns the length of the batch.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn append_batch_meta(&mut self, batch_meta: BatchMeta) {
        let _ = self.meta.insert(batch_meta);
    }

    pub fn take_meta(&mut self) -> Option<BatchMeta> {
        self.meta.take()
    }
}

impl<O> Orderable for UpdateBatch<O> {
    fn sequence_number(&self) -> SeqNo {
        self.seq_no
    }
}

impl<O> Default for UnorderedBatch<O> {
    fn default() -> Self {
        Self::new()
    }
}

impl<O> UnorderedBatch<O> {
    /// Returns a new, empty batch of requests.
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    pub fn new_with_cap(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
        }
    }

    /// Adds a new update request to the batch.
    pub fn add(&mut self, from: NodeId, session_id: SeqNo, operation_id: SeqNo, operation: O) {
        self.inner.push(Update {
            from,
            session_id,
            operation_id,
            operation,
        });
    }

    /// Returns the inner storage.
    pub fn into_inner(self) -> Vec<Update<O>> {
        self.inner
    }

    /// Returns the length of the batch.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<O> AsRef<[Update<O>]> for UpdateBatch<O> {
    fn as_ref(&self) -> &[Update<O>] {
        &self.inner[..]
    }
}

impl<O> Update<O> {
    /// Returns the inner types stored in this `Update`.
    pub fn into_inner(self) -> (NodeId, SeqNo, SeqNo, O) {
        (
            self.from,
            self.session_id,
            self.operation_id,
            self.operation,
        )
    }

    /// Returns a reference to this operation in this `Update`.
    pub fn operation(&self) -> &O {
        &self.operation
    }

    pub fn from(&self) -> NodeId {
        self.from
    }

    pub fn session_id(&self) -> SeqNo {
        self.session_id
    }

    pub fn operation_id(&self) -> SeqNo {
        self.operation_id
    }
}

impl<P> BatchReplies<P> {
    /*
        /// Returns a new, empty batch of replies.
        pub fn new() -> Self {
            Self { inner: Vec::new() }
        }
    */

    /// Returns a new, empty batch of replies, with the given capacity.
    pub fn with_capacity(n: usize) -> Self {
        Self {
            inner: Vec::with_capacity(n),
        }
    }

    /// Adds a new update reply to the batch.
    pub fn add(&mut self, to: NodeId, session_id: SeqNo, operation_id: SeqNo, payload: P) {
        self.inner.push(UpdateReply {
            to,
            session_id,
            operation_id,
            payload,
        });
    }

    pub fn push(&mut self, reply: UpdateReply<P>) {
        self.inner.push(reply);
    }

    pub fn inner(&self) -> &Vec<UpdateReply<P>> {
        &self.inner
    }

    /// Returns the inner storage.
    pub fn into_inner(self) -> Vec<UpdateReply<P>> {
        self.inner
    }

    /// Returns the length of the batch.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<O> Deref for BatchReplies<O> {
    type Target = Vec<UpdateReply<O>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<O> DerefMut for BatchReplies<O> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<O> From<Vec<UpdateReply<O>>> for BatchReplies<O> {
    fn from(value: Vec<UpdateReply<O>>) -> Self {
        Self { inner: value }
    }
}

impl<P> UpdateReply<P> {
    pub fn init(to: NodeId, session_id: SeqNo, operation_id: SeqNo, payload: P) -> Self {
        Self {
            to,
            session_id,
            operation_id,
            payload,
        }
    }

    pub fn to(&self) -> NodeId {
        self.to
    }

    /// Returns the inner types stored in this `UpdateReply`.
    pub fn into_inner(self) -> (NodeId, SeqNo, SeqNo, P) {
        (self.to, self.session_id, self.operation_id, self.payload)
    }
}
