//! Thread pool backed by `threadpool-crossbeam-channel`.
//!
//! Unlike `rayon`, this pool has no scoped execution, so [`ThreadPool::install`]
//! is emulated: the job is submitted like any other and the calling thread blocks
//! on a oneshot channel until the worker hands the result back. This is why
//! `install` requires `R: Send + 'static` here — the result has to travel across
//! the channel rather than being produced in a borrowed scope.

pub struct ThreadPool {
    inner: threadpool_crossbeam_channel::ThreadPool,
}

impl ThreadPool {
    pub fn execute<F>(&self, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.inner.execute(job)
    }

    pub fn install<F, R>(&self, job: F) -> R
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.inner.execute(move || {
            // The receiver is only dropped if the calling thread unwound, in which
            // case nobody is waiting for the value any more.
            let _ = tx.send(job());
        });

        rx.recv()
            .expect("Thread pool worker panicked before producing a result")
    }

    pub fn join(&self) {
        self.inner.join()
    }
}

pub struct Builder {
    threads: Option<usize>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { threads: None }
    }

    pub fn build(self) -> ThreadPool {
        let mut builder = threadpool_crossbeam_channel::Builder::new()
            .thread_name("Atlas-CPU-Worker".to_string());

        if let Some(n) = self.threads {
            builder = builder.num_threads(n);
        }

        ThreadPool {
            inner: builder.build(),
        }
    }

    pub fn num_threads(mut self, num_threads: usize) -> Self {
        self.threads = Some(num_threads);
        self
    }
}
