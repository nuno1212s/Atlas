use crate::channel::{RecvError, SendError, SendReturnError, TrySendError};
use crate::channel::{TryRecvError, TrySendReturnError};
use std::sync::Arc;
use std::time::Duration;

/**
Sync channels
 */
#[cfg(not(feature = "channel_sync_flume"))]
type InnerSyncChannelRx<T> = super::crossbeam::ChannelSyncRx<T>;

#[cfg(not(feature = "channel_sync_flume"))]
type InnerSyncChannelTx<T> = super::crossbeam::ChannelSyncTx<T>;

#[cfg(feature = "channel_sync_flume")]
type InnerSyncChannelRx<T> = super::flume_sync::ChannelSyncRx<T>;

#[cfg(feature = "channel_sync_flume")]
type InnerSyncChannelTx<T> = super::flume_sync::ChannelSyncTx<T>;

pub struct ChannelSyncRx<T> {
    channel_identifier: Option<Arc<str>>,
    inner: InnerSyncChannelRx<T>,
}

pub struct ChannelSyncTx<T> {
    channel_identifier: Option<Arc<str>>,
    inner: InnerSyncChannelTx<T>,
}

impl<T> ChannelSyncRx<T> {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        self.inner
            .try_recv()
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    #[inline]
    pub fn recv(&self) -> Result<T, RecvError> {
        self.inner
            .recv()
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    #[inline]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, TryRecvError> {
        self.inner
            .recv_timeout(timeout)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    pub fn name(&self) -> Option<&Arc<str>> {
        self.channel_identifier.as_ref()
    }
}

impl<T> ChannelSyncTx<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn send(&self, value: T) -> Result<(), SendError> {
        self.inner
            .send(value)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    #[inline]
    pub fn send_timeout(&self, value: T, timeout: Duration) -> Result<(), TrySendError> {
        self.inner
            .send_timeout(value, timeout)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    #[inline]
    pub fn try_send(&self, value: T) -> Result<(), TrySendError> {
        self.inner
            .try_send(value)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    pub fn name(&self) -> Option<&Arc<str>> {
        self.channel_identifier.as_ref()
    }
}

impl<T> ChannelSyncTx<T> {
    #[inline]
    pub fn send_return(&self, value: T) -> Result<(), SendReturnError<T>> {
        let value = match self.inner.try_send_return(value) {
            Ok(_) => {
                return Ok(());
            }
            Err(err) => match err {
                TrySendReturnError::Full(value, _) => {
                    tracing::error!(
                        channel = self.channel_identifier.as_deref().unwrap_or("Unknown"),
                        capacity = self.inner.capacity(),
                        current_occupation = self.inner.len(),
                        "Failed to insert into channel. Channel is full and could not directly insert, blocking",
                    );

                    value
                }
                TrySendReturnError::Disconnected(value, _) => {
                    tracing::error!("Channel is disconnected");

                    value
                }
                TrySendReturnError::Timeout(value, _) => value,
            },
        };

        self.inner
            .send_return(value)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }

    #[inline]
    pub fn try_send_return(&self, value: T) -> Result<(), TrySendReturnError<T>> {
        self.inner
            .try_send_return(value)
            .map_err(|err| err.with_channel(self.channel_identifier.clone()))
    }
}

impl<T> Clone for ChannelSyncTx<T> {
    fn clone(&self) -> Self {
        ChannelSyncTx {
            channel_identifier: self.channel_identifier.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for ChannelSyncRx<T> {
    fn clone(&self) -> Self {
        ChannelSyncRx {
            channel_identifier: self.channel_identifier.clone(),
            inner: self.inner.clone(),
        }
    }
}

#[inline]
pub fn new_bounded_sync<T>(
    bound: usize,
    name: Option<impl Into<String>>,
) -> (ChannelSyncTx<T>, ChannelSyncRx<T>) {
    let name = name.map(|string| Arc::from(string.into()));

    let (tx, rx) = {
        #[cfg(not(feature = "channel_sync_flume"))]
        {
            super::crossbeam::new_bounded(bound)
        }
        #[cfg(feature = "channel_sync_flume")]
        {
            super::flume_sync::new_bounded(bound)
        }
    };

    (
        ChannelSyncTx {
            channel_identifier: name.clone(),
            inner: tx,
        },
        ChannelSyncRx {
            channel_identifier: name,
            inner: rx,
        },
    )
}

#[inline]
pub fn new_unbounded_sync<T>(
    name: Option<impl Into<String>>,
) -> (ChannelSyncTx<T>, ChannelSyncRx<T>) {
    let name = name.map(|string| Arc::from(string.into()));

    let (tx, rx) = {
        #[cfg(not(feature = "channel_sync_flume"))]
        {
            super::crossbeam::new_unbounded()
        }
        #[cfg(feature = "channel_sync_flume")]
        {
            super::flume_sync::new_unbounded()
        }
    };

    (
        ChannelSyncTx {
            channel_identifier: name.clone(),
            inner: tx,
        },
        ChannelSyncRx {
            channel_identifier: name,
            inner: rx,
        },
    )
}

/// Drains any messages already queued on `$channel`, feeding each to
/// `$self_obj.$consumption(..)`.
///
/// Inside a [`sync_select!`] arm, prefer `recv_exhaust`, which does the same
/// thing without making you name the channel a second time.
#[macro_export]
macro_rules! exhaust_and_consume {
    ($existing_msg: expr, $channel: expr, $self_obj: expr, $consumption: ident) => {{
        $self_obj.$consumption($existing_msg)?;

        while let Ok(message) = $channel.try_recv() {
            $self_obj.$consumption(message)?;
        }

        Ok(())
    }};
    ($channel:expr, $self_obj:expr, $consumption:ident) => {
        while let Ok(message) = $channel.try_recv() {
            $self_obj.$consumption(message)?;
        }
    };
}

/// How many messages one arm may take in a single round before the round moves
/// on to the next arm.
///
/// A round drains every arm it visits, so without a cap a saturated channel
/// would keep a round running indefinitely — and the caller, which typically
/// alternates receiving with other work (the SMR replica steps its ordering
/// protocol between rounds), would never get control back. The cap bounds a
/// round at `arms * limit` messages. Anything left over stays queued for the
/// next round, which now comes around promptly.
///
/// Override per call site with a leading `limit(..);` in [`sync_select!`].
pub const DEFAULT_DRAIN_LIMIT: usize = 128;

/// Plumbing for the receive macros. Not a public API — they expand in the
/// caller's crate, so everything they touch has to be reachable from there.
///
/// This is the only place that knows which sync channel backend is compiled in.
/// The macros never receive through a selector, so all a backend has to supply
/// is a non-blocking take and a way to park; the two backends differ only in the
/// latter.
#[doc(hidden)]
pub mod __select {
    use super::ChannelSyncRx;
    use crate::channel::RecvError;
    use std::time::Instant;

    /// Identity, but its `&ChannelSyncRx<T>` parameter makes deref coercion do
    /// the work: call sites can pass either a `ChannelSyncRx<T>` place (a field)
    /// or an existing `&ChannelSyncRx<T>` (a getter's return value).
    #[inline]
    pub fn as_rx<T>(rx: &ChannelSyncRx<T>) -> &ChannelSyncRx<T> {
        rx
    }

    #[cfg(not(feature = "channel_sync_flume"))]
    mod imp {
        use super::*;

        /// One non-blocking take.
        ///
        /// `None` is the hot path — a round visits every arm, and most are empty
        /// most of the time — so it goes to the raw receiver rather than
        /// [`ChannelSyncRx::try_recv`], which would build a `TryRecvError` and
        /// clone the channel name's `Arc` just to say "nothing here".
        ///
        /// Disconnection is `Some(Err(..))`, not `None`: a closed channel has
        /// something to report, and reading it as empty would let a caller spin
        /// on a channel that can never produce again.
        #[inline]
        pub fn poll<T>(rx: &ChannelSyncRx<T>) -> Option<Result<T, RecvError>> {
            match rx.inner.raw().try_recv() {
                Ok(value) => Some(Ok(value)),
                Err(crossbeam_channel::TryRecvError::Empty) => None,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(Err(RecvError::ChannelDc {
                        channel: rx.name().cloned(),
                    }))
                }
            }
        }

        /// The take for every message after an arm's first in a round.
        ///
        /// `poll` establishes that an arm is live and reports a disconnect; once
        /// it has, the rest of that arm's drain needs neither. Returning a bare
        /// `Option<T>` keeps the per-message path down to the receive itself —
        /// this is the loop that runs once per message under load, so it is the
        /// one that has to stay lean. A disconnect ends the drain here like an
        /// empty channel would, and is picked up by the next round's `poll`.
        #[inline]
        pub fn try_next<T>(rx: &ChannelSyncRx<T>) -> Option<T> {
            rx.inner.raw().try_recv().ok()
        }

        /// Parks on the channels themselves.
        ///
        /// `Select::ready*` blocks until an operation *could* proceed and hands
        /// back only an index — it never receives. That is the whole reason this
        /// backend uses it: the round stays the only place that takes a message,
        /// so there are no per-arm slots to park values in, no operation that
        /// must be completed before its borrow can be released, and no second
        /// copy of the arm bodies.
        ///
        /// Biased, because the fairness shuffle `run_select` would otherwise do
        /// on every call costs more than the wait it precedes, and fairness is
        /// the round's job now: a round visits every arm regardless of which one
        /// woke us.
        pub type Selector<'a> = crossbeam_channel::Select<'a>;

        /// This backend keeps no state between parks — the selector borrows the
        /// receivers, so it cannot outlive a round that hands them to a
        /// `&mut self` body, and has to be rebuilt inside each wait. Carrying a
        /// zero-sized state keeps the macro's shape the same across backends
        /// without costing a `Select::new` (and its allocation) per call.
        pub struct ParkState;

        #[inline]
        pub fn new_park_state() -> ParkState {
            ParkState
        }

        #[inline]
        pub fn new_selector<'a>() -> Selector<'a> {
            crossbeam_channel::Select::new_biased()
        }

        #[inline]
        pub fn register<'a, T>(sel: &mut Selector<'a>, rx: &'a ChannelSyncRx<T>) {
            sel.recv(rx.inner.raw());
        }

        /// `false` means the deadline passed with nothing ready.
        #[inline]
        pub fn park(sel: &mut Selector<'_>, deadline: Option<Instant>) -> bool {
            match deadline {
                None => {
                    let _ = sel.ready();
                    true
                }
                Some(deadline) => sel.ready_deadline(deadline).is_ok(),
            }
        }
    }

    #[cfg(feature = "channel_sync_flume")]
    mod imp {
        use super::*;
        use std::time::Duration;

        /// See the crossbeam backend's `poll`.
        #[inline]
        pub fn poll<T>(rx: &ChannelSyncRx<T>) -> Option<Result<T, RecvError>> {
            match rx.inner.raw().try_recv() {
                Ok(value) => Some(Ok(value)),
                Err(flume::TryRecvError::Empty) => None,
                Err(flume::TryRecvError::Disconnected) => Some(Err(RecvError::ChannelDc {
                    channel: rx.name().cloned(),
                })),
            }
        }

        /// The take for every message after an arm's first in a round.
        ///
        /// `poll` establishes that an arm is live and reports a disconnect; once
        /// it has, the rest of that arm's drain needs neither. Returning a bare
        /// `Option<T>` keeps the per-message path down to the receive itself —
        /// this is the loop that runs once per message under load, so it is the
        /// one that has to stay lean. A disconnect ends the drain here like an
        /// empty channel would, and is picked up by the next round's `poll`.
        #[inline]
        pub fn try_next<T>(rx: &ChannelSyncRx<T>) -> Option<T> {
            rx.inner.raw().try_recv().ok()
        }

        /// flume's `Selector` can only wait by consuming through closures, so
        /// there is no equivalent of crossbeam's `ready` to park on. This backend
        /// backs off instead: spin briefly, then yield, then sleep. It costs a
        /// little wake latency once the sleep stage is reached, and nothing at
        /// all under load, where a round never comes up empty.
        pub struct ParkState {
            rounds: u32,
        }

        const SPIN_ROUNDS: u32 = 32;
        const YIELD_ROUNDS: u32 = 64;
        const SLEEP: Duration = Duration::from_micros(50);

        #[inline]
        pub fn new_park_state() -> ParkState {
            ParkState { rounds: 0 }
        }

        /// A no-op on this backend: there is nothing to register on.
        #[inline]
        pub fn register<T>(_state: &mut ParkState, _rx: &ChannelSyncRx<T>) {}

        /// `false` means the deadline passed. `true` only means "go round again"
        /// — unlike crossbeam's, this parker cannot know that a message arrived.
        #[inline]
        pub fn park(state: &mut ParkState, deadline: Option<Instant>) -> bool {
            if let Some(deadline) = deadline {
                if Instant::now() >= deadline {
                    return false;
                }
            }

            state.rounds += 1;

            if state.rounds <= SPIN_ROUNDS {
                std::hint::spin_loop();
            } else if state.rounds <= YIELD_ROUNDS {
                std::thread::yield_now();
            } else {
                std::thread::sleep(SLEEP);
            }

            true
        }
    }

    pub use imp::*;
}

/// Drains every one of several channels once, running each message's arm body as
/// it is taken.
///
/// The non-blocking half of [`sync_select!`], exposed on its own so it can be
/// tested and benchmarked without any parking folded in. One round visits every
/// arm in turn and takes up to `limit` messages from each, so no arm can starve
/// another however busy it is.
///
/// ```ignore
/// sync_drain! {
///     limit(32);                                          // optional
///     recv(self.work_rx) -> work => self.handle(work),
///     recv(self.timeout_rx) -> timeout => self.timeout_received(timeout),
/// }
/// ```
///
/// Every arm binds the message itself, and every body returns `Result<(), E>`;
/// the round applies `?` for you. A disconnected channel surfaces as an `Err`
/// already carrying the channel's name, so `E` must be `From<RecvError>`.
///
/// Evaluates to `Ok(())` whether or not anything was ready. The first body to
/// return `Err` ends the round, leaving every message it has not reached still
/// queued — nothing is taken from a channel until its body is about to run.
#[macro_export]
macro_rules! sync_drain {
    (limit($limit:expr); $($arms:tt)*) => {
        $crate::__atlas_recv_parse!(@start drain ($limit) $($arms)*)
    };
    ($($arms:tt)*) => {
        $crate::__atlas_recv_parse!(
            @start drain ($crate::channel::sync::DEFAULT_DRAIN_LIMIT) $($arms)*)
    };
}

/// Drains every one of several channels, parking until there is something to
/// drain.
///
/// Composed of two halves that are each usable and measurable on their own:
///
/// 1. [`sync_drain!`] — one bounded round over every arm, running bodies as
///    messages are taken. Registers nothing, allocates nothing, reads no clock.
/// 2. a parker — `Select::ready_deadline` on crossbeam, spin/yield/sleep backoff
///    on flume. It never receives; all it does is wait.
///
/// A loop that is keeping up with its inbox finishes in the first half every
/// time, and only pays for the second when it has genuinely run dry.
///
/// ```ignore
/// sync_select! {
///     limit(32);                                          // optional
///     recv(self.work_rx) -> work => self.handle(work),
///     recv_exhaust(self.timeout_rx) -> timeout => self.timeout_received(timeout),
///     default(Duration::from_millis(1)) => Ok(()),        // nothing arrived in time
/// }
/// ```
///
/// Arms bind the message and bodies return `Result<(), E>`, as in [`sync_drain!`]
/// — `recv` and `recv_exhaust` mean the same thing, since a round always drains
/// what it visits. Commas between arms are optional after a block body, matching
/// `crossbeam_channel::select!`.
///
/// # Ordering
///
/// Arms are visited in order, but order carries no priority: every round visits
/// every arm, so a busy arm cannot starve the ones after it. What arm order does
/// decide is which is served first *within* a round, and — through `limit` — how
/// much work a round does before returning to the caller.
#[macro_export]
macro_rules! sync_select {
    (limit($limit:expr); $($arms:tt)*) => {
        $crate::__atlas_recv_parse!(@start park ($limit) $($arms)*)
    };
    ($($arms:tt)*) => {
        $crate::__atlas_recv_parse!(
            @start park ($crate::channel::sync::DEFAULT_DRAIN_LIMIT) $($arms)*)
    };
}

/// Re-exported here so callers can keep writing `channel::sync::sync_select!`
/// (`#[macro_export]` alone would only expose them at the crate root).
pub use crate::{sync_drain, sync_select};

/// Front-end for the receive macros: rewrites the arms into one uniform list
/// that the emitter can expand. Backend-independent.
///
/// `$strat` is the caller's entry point — `drain` or `park` — and `($limit)` the
/// per-arm cap; both ride along untouched. Each arm is captured by a rule per
/// *separator* shape, then handed to `@arm`, which is where the per-kind meaning
/// lives.
#[doc(hidden)]
#[macro_export]
macro_rules! __atlas_recv_parse {
    (@start $strat:ident ($limit:expr) $($arms:tt)*) => {
        $crate::__atlas_recv_parse!(@munch $strat ($limit) [] $($arms)*)
    };

    // ---- `default` closes the block ----
    (@munch $strat:ident ($limit:expr) [$($acc:tt)*] default($timeout:expr) => $body:block $(,)?) => {
        $crate::__atlas_recv_emit!(@build $strat ($limit) [$($acc)*] timeout($timeout, $body))
    };
    (@munch $strat:ident ($limit:expr) [$($acc:tt)*] default($timeout:expr) => $body:expr $(,)?) => {
        $crate::__atlas_recv_emit!(@build $strat ($limit) [$($acc)*] timeout($timeout, $body))
    };

    // ---- one rule per separator shape, kind-agnostic ----
    //
    // A block body may drop the trailing comma (matching
    // `crossbeam_channel::select!`), and `macro_rules` only allows `,` or nothing
    // after an `expr` — hence exactly these four. The comma and comma-less block
    // rules must stay separate: folding them into `$(,)? $($rest:tt)*` makes the
    // comma ambiguous between the two matchers.
    (@munch $strat:ident ($limit:expr) [$($acc:tt)*]
        $kind:ident($rx:expr) -> $bind:pat => $body:block , $($rest:tt)*) => {
        $crate::__atlas_recv_parse!(@arm $kind $strat ($limit) [$($acc)*] $rx, $bind, $body, $($rest)*)
    };
    (@munch $strat:ident ($limit:expr) [$($acc:tt)*]
        $kind:ident($rx:expr) -> $bind:pat => $body:block $($rest:tt)*) => {
        $crate::__atlas_recv_parse!(@arm $kind $strat ($limit) [$($acc)*] $rx, $bind, $body, $($rest)*)
    };
    (@munch $strat:ident ($limit:expr) [$($acc:tt)*]
        $kind:ident($rx:expr) -> $bind:pat => $body:expr , $($rest:tt)*) => {
        $crate::__atlas_recv_parse!(@arm $kind $strat ($limit) [$($acc)*] $rx, $bind, $body, $($rest)*)
    };
    (@munch $strat:ident ($limit:expr) [$($acc:tt)*]
        $kind:ident($rx:expr) -> $bind:pat => $body:expr) => {
        $crate::__atlas_recv_parse!(@arm $kind $strat ($limit) [$($acc)*] $rx, $bind, $body,)
    };

    // ---- arms exhausted ----
    (@munch $strat:ident ($limit:expr) [$($acc:tt)*]) => {
        $crate::__atlas_recv_emit!(@build $strat ($limit) [$($acc)*] no_timeout)
    };

    // ---- arm kinds ----
    //
    // A round always drains the arm it visits, so `recv` and `recv_exhaust` now
    // describe the same thing. `recv_exhaust` is kept as a spelling because call
    // sites read better for saying it.
    (@arm recv $strat:ident ($limit:expr) [$($acc:tt)*]
        $rx:expr, $bind:pat, $body:expr, $($rest:tt)*) => {
        $crate::__atlas_recv_parse!(@munch $strat ($limit) [$($acc)* ($rx, $bind, $body)] $($rest)*)
    };
    (@arm recv_exhaust $strat:ident ($limit:expr) [$($acc:tt)*]
        $rx:expr, $bind:pat, $body:expr, $($rest:tt)*) => {
        $crate::__atlas_recv_parse!(@munch $strat ($limit) [$($acc)* ($rx, $bind, $body)] $($rest)*)
    };
    (@arm $other:ident $strat:ident ($limit:expr) [$($acc:tt)*]
        $rx:expr, $bind:pat, $body:expr, $($rest:tt)*) => {
        compile_error!(concat!(
            "sync_select!: unknown arm `", stringify!($other),
            "`; expected `recv`, `recv_exhaust` or `default`"
        ))
    };
}

/// One bounded round over every arm, running each body as its message is taken.
///
/// Evaluates to `Result<bool, E>` — the `bool` says whether anything at all was
/// serviced, which is what tells [`sync_select!`] whether it has to park.
///
/// Nothing is dequeued until its body is about to run, so an `Err` ending the
/// round leaves every message the round did not reach still in its channel.
/// `$rx` is re-evaluated per message for the same reason `poll` takes an owned
/// value out: the receiver borrow has to be dead before a body that takes
/// `&mut self` runs.
#[doc(hidden)]
#[macro_export]
macro_rules! __atlas_drain_round {
    (($limit:expr) [$(($rx:expr, $bind:pat, $body:expr))*]) => {
        '__atlas_round: {
            let mut __atlas_serviced = false;
            let mut __atlas_dc = ::core::option::Option::None;

            $({
                let mut __atlas_budget: usize = $limit;
                let mut __atlas_first = true;

                while __atlas_budget > 0 {
                    // Only the first take of an arm goes through `poll`, which is
                    // the one that has to distinguish empty from disconnected.
                    // The rest — the per-message path under load — take the lean
                    // route.
                    let __atlas_taken = if __atlas_first {
                        __atlas_first = false;

                        $crate::channel::sync::__select::poll(
                            $crate::channel::sync::__select::as_rx(&$rx),
                        )
                    } else {
                        match $crate::channel::sync::__select::try_next(
                            $crate::channel::sync::__select::as_rx(&$rx),
                        ) {
                            ::core::option::Option::Some(__atlas_v) => {
                                ::core::option::Option::Some(::core::result::Result::Ok(__atlas_v))
                            }
                            ::core::option::Option::None => ::core::option::Option::None,
                        }
                    };

                    let $bind = match __atlas_taken {
                        ::core::option::Option::None => break,
                        // A dropped sender ends this arm's drain the way an empty
                        // channel does, rather than ending the round: the arms
                        // after it may still have messages, and reporting the
                        // disconnect while there is work left would strand it.
                        // It is held over and returned below, but only once the
                        // round has nothing to show for itself — by which point
                        // the caller has drained everything the dead channel
                        // ever delivered.
                        ::core::option::Option::Some(::core::result::Result::Err(__atlas_e)) => {
                            __atlas_dc = ::core::option::Option::Some(
                                ::core::convert::From::from(__atlas_e),
                            );
                            break;
                        }
                        ::core::option::Option::Some(::core::result::Result::Ok(__atlas_v)) => {
                            __atlas_v
                        }
                    };

                    __atlas_serviced = true;
                    __atlas_budget -= 1;

                    // A body failing *is* terminal for the round. Nothing has
                    // been taken from the arms it did not reach, so their
                    // messages stay queued.
                    if let ::core::result::Result::Err(__atlas_e) = $body {
                        break '__atlas_round ::core::result::Result::Err(__atlas_e);
                    }
                }
            })*

            match (__atlas_serviced, __atlas_dc) {
                (true, _) => ::core::result::Result::Ok(true),
                (false, ::core::option::Option::Some(__atlas_e)) => {
                    ::core::result::Result::Err(__atlas_e)
                }
                (false, ::core::option::Option::None) => ::core::result::Result::Ok(false),
            }
        }
    };
}

/// Back-end for the receive macros.
///
/// The arm bodies are emitted exactly once, inside the round — the parker never
/// receives, so there is no second place they would have to appear.
#[doc(hidden)]
#[macro_export]
macro_rules! __atlas_recv_emit {
    // ---- one round, no parking ----
    (@build drain ($limit:expr) [$($arms:tt)*] no_timeout) => {
        match $crate::__atlas_drain_round!(($limit) [$($arms)*]) {
            ::core::result::Result::Ok(_) => ::core::result::Result::Ok(()),
            ::core::result::Result::Err(__atlas_e) => ::core::result::Result::Err(__atlas_e),
        }
    };
    (@build drain ($limit:expr) [$($arms:tt)*] timeout($timeout:expr, $default:expr)) => {
        compile_error!("sync_drain!: has nothing to wait for; drop the `default` arm, or use sync_select!")
    };

    // ---- round, then park, until something is serviced or the deadline passes ----
    (@build park ($limit:expr) [$(($rx:expr, $bind:pat, $body:expr))*] $($mode:tt)*) => {{
        let mut __atlas_deadline: ::core::option::Option<::std::time::Instant> =
            ::core::option::Option::None;
        let mut __atlas_park_state = $crate::channel::sync::__select::new_park_state();

        loop {
            match $crate::__atlas_drain_round!(($limit) [$(($rx, $bind, $body))*]) {
                ::core::result::Result::Err(__atlas_e) => {
                    break ::core::result::Result::Err(__atlas_e);
                }
                ::core::result::Result::Ok(true) => break ::core::result::Result::Ok(()),
                ::core::result::Result::Ok(false) => {}
            }

            // Only now, having found nothing, is a deadline worth the clock read
            // — and only the first time round.
            $crate::__atlas_recv_emit!(@deadline __atlas_deadline, $($mode)*);

            if !$crate::__atlas_park!(__atlas_park_state, __atlas_deadline, [$($rx),*]) {
                break $crate::__atlas_recv_emit!(@fallback $($mode)*);
            }
        }
    }};

    (@deadline $deadline:ident, no_timeout) => {};
    (@deadline $deadline:ident, timeout($timeout:expr, $default:expr)) => {
        if $deadline.is_none() {
            $deadline = ::core::option::Option::Some(::std::time::Instant::now() + $timeout);
        }
    };

    (@fallback no_timeout) => {
        unreachable!("parking reported a deadline it was never given")
    };
    (@fallback timeout($timeout:expr, $default:expr)) => { $default };
}

/// Parking for `crossbeam_channel`, which can wait on the channels themselves.
///
/// The receivers are borrowed only for the duration of the wait, so a body that
/// takes `&mut self` is free to run once the round resumes.
#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "channel_sync_flume"))]
macro_rules! __atlas_park {
    ($state:ident, $deadline:expr, [$($rx:expr),*]) => {{
        let _ = &$state;

        let mut __atlas_sel = $crate::channel::sync::__select::new_selector();

        $( $crate::channel::sync::__select::register(
            &mut __atlas_sel,
            $crate::channel::sync::__select::as_rx(&$rx),
        ); )*

        $crate::channel::sync::__select::park(&mut __atlas_sel, $deadline)
    }};
}

/// Parking for `flume`, which has no way to wait without consuming — so this
/// backs off instead of watching the channels. State lives in `$parker`, which
/// is why it is threaded through at all.
#[doc(hidden)]
#[macro_export]
#[cfg(feature = "channel_sync_flume")]
macro_rules! __atlas_park {
    ($state:ident, $deadline:expr, [$($rx:expr),*]) => {
        $crate::channel::sync::__select::park(&mut $state, $deadline)
    };
}
