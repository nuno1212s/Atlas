//! This crate, `atlas-common`, provides the pluggable backends the rest of
//! Atlas is built on: the async runtime, thread pool, sockets, channels, hashing,
//! the `HashMap` hasher, and the persistent key-value store.
//!
//! # Choosing a backend
//!
//! Every group has a default that is selected by the *absence* of a feature
//! flag, so a plain `cargo build` gets the standard stack and there is nothing
//! to configure to get started. To change one, enable its flag anywhere in the
//! dependency graph -- including from the final binary's `Cargo.toml`:
//!
//! ```toml
//! atlas-common = { path = "...", features = ["threadpool_crossbeam"] }
//! ```
//!
//! | group                 | default | override with                                  |
//! |-----------------------|---------|------------------------------------------------|
//! | async runtime         | tokio   | `async_runtime_async_std`                       |
//! | socket                | tokio   | `socket_async_std_tcp`, `socket_rio_tcp`        |
//! | thread pool           | rayon   | `threadpool_crossbeam`                          |
//! | hash                  | blake3  | `crypto_hash_ring_sha2`                         |
//! | async channel         | flume   | `channel_async_channel_mpmc`                    |
//! | sync channel          | crossbeam | `channel_sync_flume`                          |
//! | dump queue            | mqueue  | `channel_custom_dump_lfb`                       |
//! | `RandomState`         | fxhash  | `collections_randomstate_{std,twox_hash,gxhash}` |
//! | persistent db         | sled    | `persistent_db_rocksdb`, `persistent_db_disabled` |
//!
//! There is deliberately **no backend in `default`**. Cargo features are
//! additive and unioned across the whole dependency graph, so a positive default
//! could only be switched off with `default-features = false` on every one of
//! the ~20 edges that reach this crate. Expressing the default as "no override
//! selected" instead means a single flag, anywhere, wins.
//!
//! Selecting two backends from one group is rejected by `build.rs` with a
//! readable message rather than a wall of duplicate-definition errors.
//!
//! Signing (`ring` Ed25519), the mixed channel and the multi-dump channel have a
//! single implementation each and so have no flag at all.
//!
//! `serialize_serde` is the one remaining default feature: it is a cross-cutting
//! derive gate rather than a backend choice, and it is forwarded by roughly a
//! dozen sibling crates.

use crate::error::*;
use crate::globals::Flag;
use tracing::{debug, instrument};

/// Re-exported for `sync_select!`, which expands in the caller's crate where
/// `flume` is not a direct dependency.
#[cfg(feature = "channel_sync_flume")]
#[doc(hidden)]
pub use flume as __flume;

pub mod async_runtime;
pub mod channel;
pub mod circuit_breaker;
pub mod collections;
pub mod config_utils;
pub mod crypto;
pub mod error;
pub mod globals;
pub mod maybe_vec;
pub mod node_id;
pub mod ordering;
pub mod peer_addr;
pub mod persistentdb;
pub mod phantom;
pub mod prng;
pub mod serialization_helper;
pub mod socket;
pub mod system_params;
pub mod threadpool;

static INITIALIZED: Flag = Flag::new();

/// Configure the init process of the library.
#[derive(Debug)]
pub struct InitConfig {
    /// Number of threads used by the async runtime.
    pub async_threads: usize,
    /// Number of threads used by the thread pool.
    pub threadpool_threads: usize,
}

/// Handle to the global data.
///
/// When dropped, the data is deinitialized.
#[repr(transparent)]
pub struct InitGuard;

/// Initializes global data.
///
/// Should always be called before other methods, otherwise runtime
/// panics may ensue.
///
/// # Safety
///
/// Safe when this is called once, and before all others.
/// Returns init guard that will automatically call drop when
/// dropped.
/// Keep [InitGuard] within scope for the execution of the program.
#[instrument()]
pub unsafe fn init(c: InitConfig) -> Result<Option<InitGuard>> {
    if INITIALIZED.test() {
        return Ok(None);
    }

    threadpool::init(c.threadpool_threads)?;
    async_runtime::init(c.async_threads)?;

    debug!(
        "Async threads {}, sync threads {}",
        c.async_threads, c.threadpool_threads
    );

    unsafe {
        socket::init()?;
    }
    INITIALIZED.set();
    Ok(Some(InitGuard))
}

impl Drop for InitGuard {
    fn drop(&mut self) {
        unsafe { drop().unwrap() }
    }
}

/// # Safety
/// Safe when called after [init].
/// Ideally, use [InitGuard] to control access to this function
#[instrument]
unsafe fn drop() -> Result<()> {
    INITIALIZED.unset();
    unsafe {
        threadpool::drop()?;
        async_runtime::drop()?;
        socket::drop()?;
    }
    Ok(())
}
