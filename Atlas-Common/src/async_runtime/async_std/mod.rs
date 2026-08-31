use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::error::*;

pub struct JoinHandle<T> {
    inner: ::async_std::task::JoinHandle<T>,
}

#[derive(Debug)]
pub struct Runtime;

pub fn init(num_threads: usize) -> Result<Runtime> {
    // SAFETY: `init` is documented as being called once, before any other Atlas
    // function and therefore before any other thread exists to observe the
    // environment. async-std reads this variable when it first starts its pool.
    unsafe {
        std::env::set_var("ASYNC_STD_THREAD_COUNT", format!("{num_threads}"));
    }

    Ok(Runtime)
}

impl Runtime {
    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let inner = ::async_std::task::spawn(future);
        JoinHandle { inner }
    }

    pub fn spawn_blocking<F, R>(&self, job: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let inner = ::async_std::task::spawn_blocking(job);
        JoinHandle { inner }
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        ::async_std::task::block_on(future)
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx).map(Ok)
    }
}
