use crate::serialize::ApplicationData;
use atlas_common::error::*;
use atlas_core::execution::requests::{
    IncrementableUpdateBatch, ReplyBatch, UnorderedUpdateBatch, UpdateBatch,
};

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
        requests: UnorderedUpdateBatch<Request<Self, S>>,
    ) -> ReplyBatch<Reply<Self, S>> {
        let mut reply_batch = ReplyBatch::new_with_cap(requests.len());

        for unordered_req in requests.into_inner() {
            let (update_info, req) = unordered_req.into_inner();
            let reply = self.unordered_execution(state, req);
            reply_batch.add(update_info, reply);
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
    ) -> ReplyBatch<Reply<Self, S>> {
        let mut reply_batch = ReplyBatch::new_with_cap(batch.len());

        for update in batch.into_inner().1 {
            let (update_info, req) = update.into_inner();

            let reply = self.update(state, req);
            reply_batch.add(update_info, reply);
        }

        reply_batch
    }
}
