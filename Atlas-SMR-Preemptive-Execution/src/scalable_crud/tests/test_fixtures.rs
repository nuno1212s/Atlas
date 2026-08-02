//! Test fixtures for `scalable_crud` tests.
//!
//! Reuses `MapState`, `MapApp`, `NoopNode`, `make_batch`, `RECV_TIMEOUT` and `KvOp`
//! from the `single_threaded_crud` test fixtures, and provides a `spawn_worker` that
//! creates a `ScalableCachingPreemptiveWorker` instead.

use std::sync::Arc;
use std::time::Duration;

use atlas_common::channel::sync::ChannelSyncRx;
use atlas_smr_application::state::monolithic_state::{AppStateMessage, InstallStateMessage};
use atlas_smr_execution::repliers::FollowerReplier;

use crate::exec_handle::PreemptiveExecutorHandle;
use crate::scalable_crud::{init_executor, init_handle};

// Re-export everything from the single_threaded_crud test fixtures.
pub use crate::single_threaded_crud::tests::test_fixtures::{
    KvOp, MapApp, MapState, NoopNode, make_batch,
};

/// Timeout for all blocking channel receives in integration tests.
pub const RECV_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn the scalable CRUD preemptive worker and return the send handle together with the
/// state-install tx and checkpoint rx so tests can drive state transfer and
/// observe confirmed-state snapshots.
pub fn spawn_worker() -> (
    PreemptiveExecutorHandle<KvOp>,
    atlas_common::channel::sync::ChannelSyncTx<InstallStateMessage<MapState>>,
    ChannelSyncRx<AppStateMessage<MapState>>,
) {
    let handle = init_handle::<MapApp, MapState>();
    let (state_tx, checkpoint_rx) = init_executor::<MapApp, MapState, NoopNode, FollowerReplier>(
        handle.get_request_receiver().clone(),
        None,
        MapApp,
        Arc::new(NoopNode),
    )
    .expect("failed to spawn scalable CRUD preemptive worker");
    (handle, state_tx, checkpoint_rx)
}
