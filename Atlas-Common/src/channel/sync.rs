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

/// Plumbing for [`sync_select!`]. Not a public API — the macro expands in the
/// caller's crate, so everything it touches has to be reachable from there.
///
/// This is the only place that knows which sync channel backend is compiled in.
/// `crossbeam_channel::Select` is index-based, while `flume::Selector` is a
/// consuming builder of closures that must all be live at once; the two are
/// reconciled here so that [`sync_select!`] itself is backend-agnostic.
#[doc(hidden)]
pub mod __select {
    use super::ChannelSyncRx;
    use crate::channel::RecvError;

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
        use std::time::Duration;

        pub type Selector<'a> = crossbeam_channel::Select<'a>;

        #[inline]
        pub fn new_selector<'a>() -> Selector<'a> {
            crossbeam_channel::Select::new()
        }

        #[inline]
        pub fn register<'a, T>(sel: &mut Selector<'a>, rx: &'a ChannelSyncRx<T>) -> usize {
            sel.recv(rx.inner.raw())
        }

        /// Blocks until one of the registered channels is ready, returning the
        /// index of the winner along with the operation that must be completed.
        #[inline]
        pub fn wait<'a>(sel: &mut Selector<'a>) -> crossbeam_channel::SelectedOperation<'a> {
            sel.select()
        }

        #[inline]
        pub fn wait_timeout<'a>(
            sel: &mut Selector<'a>,
            timeout: Duration,
        ) -> Option<crossbeam_channel::SelectedOperation<'a>> {
            sel.select_timeout(timeout).ok()
        }

        /// Completes the selected operation against the receiver it was
        /// registered with, normalizing the backend error and attaching the
        /// channel's name.
        #[inline]
        pub fn complete<T>(
            op: crossbeam_channel::SelectedOperation<'_>,
            rx: &ChannelSyncRx<T>,
        ) -> Result<T, RecvError> {
            op.recv(rx.inner.raw())
                .map_err(|err| RecvError::from(err).with_channel(rx.name().cloned()))
        }
    }

    #[cfg(feature = "channel_sync_flume")]
    mod imp {
        use super::*;

        pub use flume::Selector;

        /// Normalizes flume's receive result and attaches the channel's name.
        #[inline]
        pub fn map_recv<T>(
            rx: &ChannelSyncRx<T>,
            result: Result<T, flume::RecvError>,
        ) -> Result<T, RecvError> {
            result.map_err(|err| RecvError::from(err).with_channel(rx.name().cloned()))
        }

        #[inline]
        pub fn raw<T>(rx: &ChannelSyncRx<T>) -> &flume::Receiver<T> {
            rx.inner.raw()
        }

        /// `Err` means the wait timed out.
        #[inline]
        pub fn timed_out(result: Result<(), flume::select::SelectError>) -> bool {
            result.is_err()
        }
    }

    pub use imp::*;
}

/// Blocks until one of several channels has a message, then runs that arm.
///
/// Backend-agnostic: arms name an `atlas_common::channel::sync::ChannelSyncRx`
/// directly, and bindings carry this crate's own [`RecvError`], already tagged
/// with the channel's name. Nothing here exposes `crossbeam_channel` or `flume`.
///
/// ```ignore
/// sync_select! {
///     // binds Result<T, RecvError>
///     recv(self.work_rx) -> msg => match msg {
///         Ok(work) => self.handle(work),
///         Err(_) => return,               // channel closed
///     },
///     // binds T; applies `?` for you, then drains whatever else is queued
///     recv_exhaust(self.timeout_rx) -> timeout => self.timeout_received(timeout),
///     // optional; runs when nothing was ready within the timeout
///     default(Duration::from_millis(1)) => Ok(()),
/// }
/// ```
///
/// `recv` binds the `Result` because some callers need to see the disconnect;
/// `recv_exhaust` binds the unwrapped message, since draining a channel only
/// makes sense once the first receive succeeded. Commas between arms are
/// optional after a block body, matching `crossbeam_channel::select!`.
///
/// Selection is unbiased: earlier arms get no priority over later ones.
#[macro_export]
macro_rules! sync_select {
    ($($arms:tt)*) => {
        $crate::__atlas_select_parse!(@start $($arms)*)
    };
}

/// Re-exported here so callers can keep writing `channel::sync::sync_select!`
/// (`#[macro_export]` alone would only expose it at the crate root).
pub use crate::sync_select;

/// Front-end for [`sync_select!`]: rewrites the arms into one uniform list that
/// the backend emitter can expand. Backend-independent.
///
/// Each arm is captured by a rule per *separator* shape, then handed to `@arm`,
/// which is where the per-kind meaning lives — so `recv_exhaust`'s desugaring is
/// written once rather than once per shape.
#[doc(hidden)]
#[macro_export]
macro_rules! __atlas_select_parse {
    // Seeds one (slot, receiver, index) identifier triple per arm. Extend the
    // pool if a call site ever needs more than twelve arms.
    (@start $($arms:tt)*) => {
        $crate::__atlas_select_parse!(@munch []
            [(__atlas_s0 __atlas_r0 __atlas_i0) (__atlas_s1 __atlas_r1 __atlas_i1)
             (__atlas_s2 __atlas_r2 __atlas_i2) (__atlas_s3 __atlas_r3 __atlas_i3)
             (__atlas_s4 __atlas_r4 __atlas_i4) (__atlas_s5 __atlas_r5 __atlas_i5)
             (__atlas_s6 __atlas_r6 __atlas_i6) (__atlas_s7 __atlas_r7 __atlas_i7)
             (__atlas_s8 __atlas_r8 __atlas_i8) (__atlas_s9 __atlas_r9 __atlas_i9)
             (__atlas_s10 __atlas_r10 __atlas_i10) (__atlas_s11 __atlas_r11 __atlas_i11)]
            $($arms)*)
    };

    // ---- `default` closes the block ----
    (@munch [$($acc:tt)*] [$($ids:tt)*] default($timeout:expr) => $body:block $(,)?) => {
        $crate::__atlas_select_emit!(@build [$($acc)*] timeout($timeout, $body))
    };
    (@munch [$($acc:tt)*] [$($ids:tt)*] default($timeout:expr) => $body:expr $(,)?) => {
        $crate::__atlas_select_emit!(@build [$($acc)*] timeout($timeout, $body))
    };

    // ---- one rule per separator shape, kind-agnostic ----
    //
    // A block body may drop the trailing comma (matching
    // `crossbeam_channel::select!`), and `macro_rules` only allows `,` or nothing
    // after an `expr` — hence exactly these four. Note the comma and comma-less
    // block rules must stay separate: folding them into `$(,)? $($rest:tt)*`
    // makes the comma ambiguous between the two matchers.
    (@munch [$($acc:tt)*] [$slot:tt $($ids:tt)*]
        $kind:ident($rx:expr) -> $bind:pat => $body:block , $($rest:tt)*) => {
        $crate::__atlas_select_parse!(@arm $kind [$($acc)*] $slot [$($ids)*] $rx, $bind, $body, $($rest)*)
    };
    (@munch [$($acc:tt)*] [$slot:tt $($ids:tt)*]
        $kind:ident($rx:expr) -> $bind:pat => $body:block $($rest:tt)*) => {
        $crate::__atlas_select_parse!(@arm $kind [$($acc)*] $slot [$($ids)*] $rx, $bind, $body, $($rest)*)
    };
    (@munch [$($acc:tt)*] [$slot:tt $($ids:tt)*]
        $kind:ident($rx:expr) -> $bind:pat => $body:expr , $($rest:tt)*) => {
        $crate::__atlas_select_parse!(@arm $kind [$($acc)*] $slot [$($ids)*] $rx, $bind, $body, $($rest)*)
    };
    (@munch [$($acc:tt)*] [$slot:tt $($ids:tt)*]
        $kind:ident($rx:expr) -> $bind:pat => $body:expr) => {
        $crate::__atlas_select_parse!(@arm $kind [$($acc)*] $slot [$($ids)*] $rx, $bind, $body,)
    };

    // ---- arms exhausted ----
    (@munch [$($acc:tt)*] [$($ids:tt)*]) => {
        $crate::__atlas_select_emit!(@build [$($acc)*] no_timeout)
    };

    // ---- what each arm kind means, written once ----
    (@arm recv [$($acc:tt)*] ($s:ident $r:ident $i:ident) [$($ids:tt)*]
        $rx:expr, $bind:pat, $body:expr, $($rest:tt)*) => {
        $crate::__atlas_select_parse!(@munch
            [$($acc)* ($s $r $i, $rx, $bind, $body)] [$($ids)*] $($rest)*)
    };

    // Unwrap, run, then drain whatever else is already queued. `$rx` is emitted
    // a second time here: re-evaluating it per iteration keeps the drain's shared
    // borrow from colliding with a body that takes `&mut self`.
    (@arm recv_exhaust [$($acc:tt)*] ($s:ident $r:ident $i:ident) [$($ids:tt)*]
        $rx:expr, $bind:pat, $body:expr, $($rest:tt)*) => {
        $crate::__atlas_select_parse!(@munch
            [$($acc)* ($s $r $i, $rx, __atlas_first, {
                let $bind = __atlas_first?;
                $body?;
                while let Ok($bind) = $rx.try_recv() { $body?; }
                Ok(())
            })]
            [$($ids)*] $($rest)*)
    };

    (@arm $other:ident [$($acc:tt)*] $slot:tt [$($ids:tt)*]
        $rx:expr, $bind:pat, $body:expr, $($rest:tt)*) => {
        compile_error!(concat!(
            "sync_select!: unknown arm `", stringify!($other),
            "`; expected `recv`, `recv_exhaust` or `default`"
        ))
    };
}

/// Back-end for [`sync_select!`], in two phases:
///
/// * **phase 1** ([`__atlas_select_run`], the only backend-specific part)
///   registers the receivers and parks the winner's `Result<T, RecvError>` in its
///   own slot. The receiver borrows live only inside that block.
/// * **phase 2** runs the winning arm's body. Because phase 1's borrows are
///   already released, bodies are free to take `&mut self` — which is the whole
///   reason for the split, since every real call site does exactly that, and
///   `flume::Selector` needs all arm closures live simultaneously.
#[doc(hidden)]
#[macro_export]
macro_rules! __atlas_select_emit {
    (@build [$(($s:ident $r:ident $i:ident, $rx:expr, $bind:pat, $body:expr))*] $($mode:tt)*) => {{
        $( let mut $s = ::core::option::Option::None; )*

        {
            $( let $r = $crate::channel::sync::__select::as_rx(&$rx); )*

            $crate::__atlas_select_run!([$( ($s $r $i) )*], $($mode)*);
        }

        $( if let ::core::option::Option::Some(__atlas_v) = $s.take() {
            let $bind = __atlas_v;
            $body
        } else )* {
            $crate::__atlas_select_emit!(@fallback $($mode)*)
        }
    }};

    (@fallback no_timeout) => {
        unreachable!("the selector completed without producing a value")
    };
    (@fallback timeout($timeout:expr, $default:expr)) => { $default };
}

/// Phase 1 for `crossbeam_channel`, whose `Select` is index-based.
#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "channel_sync_flume"))]
macro_rules! __atlas_select_run {
    (@wait $sel:ident, no_timeout) => {
        ::core::option::Option::Some($crate::channel::sync::__select::wait(&mut $sel))
    };
    (@wait $sel:ident, timeout($timeout:expr, $default:expr)) => {
        $crate::channel::sync::__select::wait_timeout(&mut $sel, $timeout)
    };

    ([$(($s:ident $r:ident $i:ident))*], $($mode:tt)*) => {
        let mut __atlas_sel = $crate::channel::sync::__select::new_selector();
        $( let $i = $crate::channel::sync::__select::register(&mut __atlas_sel, $r); )*

        if let ::core::option::Option::Some(__atlas_op) =
            $crate::__atlas_select_run!(@wait __atlas_sel, $($mode)*)
        {
            let __atlas_idx = __atlas_op.index();

            $( if __atlas_idx == $i {
                $s = ::core::option::Option::Some(
                    $crate::channel::sync::__select::complete(__atlas_op, $r),
                );
            } else )* {
                unreachable!("selector returned an index that was never registered")
            }
        }
    };
}

/// Phase 1 for `flume`, whose `Selector` is a consuming builder of closures that
/// must all be live at once and share one return type — so each closure only
/// parks its value, and the bodies run later, in phase 2.
#[doc(hidden)]
#[macro_export]
#[cfg(feature = "channel_sync_flume")]
macro_rules! __atlas_select_run {
    (@finish $sel:ident, no_timeout) => { $sel.wait() };
    (@finish $sel:ident, timeout($timeout:expr, $default:expr)) => {
        { let _ = $sel.wait_timeout($timeout); }
    };

    ([$(($s:ident $r:ident $i:ident))*], $($mode:tt)*) => {
        let __atlas_sel = $crate::channel::sync::__select::Selector::new()
            $( .recv($crate::channel::sync::__select::raw($r), {
                let slot = &mut $s;
                move |received| {
                    *slot = ::core::option::Option::Some(
                        $crate::channel::sync::__select::map_recv($r, received),
                    );
                }
            }) )*;

        $crate::__atlas_select_run!(@finish __atlas_sel, $($mode)*);
    };
}
