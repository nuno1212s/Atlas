//! Abstractions over different async runtimes in the Rust ecosystem.

#[cfg(feature = "async_runtime_tokio")]
mod tokio;

#[cfg(feature = "async_runtime_async_std")]
mod async_std;

use crate::error::Result;
use anyhow::Context;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context as Cntx, Poll};

#[cfg(feature = "async_runtime_tokio")]
static RUNTIME: OnceLock<tokio::Runtime> = OnceLock::new();

#[cfg(feature = "async_runtime_async_std")]
static RUNTIME: OnceLock<async_std::Runtime> = OnceLock::new();

macro_rules! runtime {
    () => {
        RUNTIME.get().expect("Async runtime wasn't initialized")
    };
}

/// A `JoinHandle` represents a future that can be awaited on.
///
/// It resolves to a value of `T` when the future completes,
/// i.e. when the underlying async task associated with the
/// `JoinHandle` completes.
pub struct JoinHandle<T> {
    #[cfg(feature = "async_runtime_tokio")]
    inner: tokio::JoinHandle<T>,

    #[cfg(feature = "async_runtime_async_std")]
    inner: async_std::JoinHandle<T>,
}

/// This function initializes the async runtime.
///
/// It should be called once before the core protocol starts executing.
///
/// # Safety
/// This is safe when it's the first called function and when it's only called once
pub fn init(num_threads: usize) -> Result<()> {
    #[cfg(feature = "async_runtime_tokio")]
    {
        let runtime = tokio::init(num_threads)?;

        RUNTIME
            .set(runtime)
            .map_err(|err| anyhow::anyhow!("Failed to set Tokio runtime: {err:?}"))
    }

    #[cfg(feature = "async_runtime_async_std")]
    {
        let runtime = async_std::init(num_threads);

        RUNTIME
            .set(runtime)
            .map_err(|err| anyhow::anyhow!("Failed to set Async Runtime runtime: {:?}", err))
    }
}

/// This function drops the async runtime.
///
/// It shouldn't be needed to be called manually called, as the
/// `InitGuard` should take care of calling this.
///
/// # Safety
/// Safe when called after [init()]
pub unsafe fn drop() -> Result<()> {
    Ok(())
}

/// Spawns a new task `F` into the async runtime's thread pool.
///
/// A handle to the future `JoinHandle` is returned, which can be
/// awaited on, to resolve the value returned by `F`.
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let inner = runtime!().spawn(future);
    JoinHandle { inner }
}

pub fn spawn_blocking<F, R>(future: F) -> JoinHandle<F::Output>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let inner = runtime!().spawn_blocking(future);

    JoinHandle { inner }
}

/// Blocks on a future `F` until it completes.
pub fn block_on<F: Future>(future: F) -> F::Output {
    runtime!().block_on(future)
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Cntx<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner)
            .poll(cx)
            .map(|result| result.context("Failed to join handle"))
    }
}

/// Yields execution back to the async runtime.
pub async fn yield_now() {
    struct YieldNow {
        yielded: bool,
    }

    impl Future for YieldNow {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Cntx<'_>) -> Poll<()> {
            if self.yielded {
                return Poll::Ready(());
            }
            self.yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    YieldNow { yielded: false }.await;
}
