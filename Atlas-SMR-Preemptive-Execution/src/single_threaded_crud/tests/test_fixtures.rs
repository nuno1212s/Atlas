//! Shared test fixtures for `single_threaded_crud` tests.
//!
//! Provides the minimal CRUD application types, a no-op network node, and the
//! helpers used by both the unit tests and the integration tests.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use atlas_common::channel::sync::ChannelSyncRx;
use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::{IncrementableUpdateBatch, UpdateBatch, UpdateInfo};
use atlas_smr_application::app::Application;
use atlas_smr_application::serialize::ApplicationData;
use atlas_smr_application::state::monolithic_state::{
    AppStateMessage, InstallStateMessage, MonolithicState,
};
use atlas_smr_core::SMRReply;
use atlas_smr_core::execution::reply::{ReplyNode, RequestType};
use atlas_smr_execution::crud_states::{CRUDApplication, CRUDState};
use atlas_smr_execution::repliers::FollowerReplier;

use crate::exec_handle::PreemptiveExecutorHandle;
use crate::single_threaded_crud::{init_executor, init_handle};

// ---------------------------------------------------------------------------
// MapState — HashMap-backed CRUD state
// ---------------------------------------------------------------------------

/// A flat key→value store keyed by bytes.  All CRUD operations target a single
/// implicit "default" column — the column name is ignored.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MapState(pub std::collections::HashMap<Vec<u8>, Vec<u8>>);

impl CRUDState for MapState {
    fn read(&self, _column: &str, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get(key).cloned()
    }

    fn create(&mut self, _column: &str, key: &[u8], value: &[u8]) -> bool {
        if self.0.contains_key(key) {
            return false;
        }
        self.0.insert(key.to_vec(), value.to_vec());
        true
    }

    fn update(&mut self, _column: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>> {
        self.0.insert(key.to_vec(), value.to_vec())
    }

    fn delete(&mut self, _column: &str, key: &[u8]) -> Option<Vec<u8>> {
        self.0.remove(key)
    }
}

impl MonolithicState for MapState {
    fn serialize_state<W: Write>(mut w: W, s: &Self) -> atlas_common::error::Result<()> {
        let entries: Vec<(&Vec<u8>, &Vec<u8>)> = s.0.iter().collect();
        w.write_all(&(entries.len() as u32).to_le_bytes())?;
        for (k, v) in &entries {
            w.write_all(&(k.len() as u32).to_le_bytes())?;
            w.write_all(k)?;
            w.write_all(&(v.len() as u32).to_le_bytes())?;
            w.write_all(v)?;
        }
        Ok(())
    }

    fn deserialize_state<R: Read>(mut r: R) -> atlas_common::error::Result<Self> {
        let mut len_buf = [0u8; 4];
        r.read_exact(&mut len_buf)?;
        let count = u32::from_le_bytes(len_buf) as usize;
        let mut map = std::collections::HashMap::new();
        for _ in 0..count {
            let mut kl = [0u8; 4];
            r.read_exact(&mut kl)?;
            let mut k = vec![0u8; u32::from_le_bytes(kl) as usize];
            r.read_exact(&mut k)?;
            let mut vl = [0u8; 4];
            r.read_exact(&mut vl)?;
            let mut v = vec![0u8; u32::from_le_bytes(vl) as usize];
            r.read_exact(&mut v)?;
            map.insert(k, v);
        }
        Ok(MapState(map))
    }
}

// ---------------------------------------------------------------------------
// MapAppData — minimal ApplicationData
// ---------------------------------------------------------------------------

/// Request = `(key, Some(value))` for a write, `(key, None)` for a delete.
/// Reply   = the previous value for that key, or `None`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct MapAppData;

impl ApplicationData for MapAppData {
    type Request = (Vec<u8>, Option<Vec<u8>>);
    type Reply = Option<Vec<u8>>;

    fn serialize_request<W: Write>(_: W, _: &Self::Request) -> atlas_common::error::Result<()> {
        Ok(())
    }
    fn deserialize_request<R: Read>(_: R) -> atlas_common::error::Result<Self::Request> {
        Ok((vec![], None))
    }
    fn serialize_reply<W: Write>(_: W, _: &Self::Reply) -> atlas_common::error::Result<()> {
        Ok(())
    }
    fn deserialize_reply<R: Read>(_: R) -> atlas_common::error::Result<Self::Reply> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// MapApp — Application + CRUDApplication
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct MapApp;

impl Application<MapState> for MapApp {
    type AppData = MapAppData;

    fn initial_state() -> atlas_common::error::Result<MapState> {
        Ok(MapState::default())
    }

    fn unordered_execution(
        &self,
        state: &MapState,
        req: (Vec<u8>, Option<Vec<u8>>),
    ) -> Option<Vec<u8>> {
        state.read("default", &req.0)
    }

    fn update(&self, state: &mut MapState, req: (Vec<u8>, Option<Vec<u8>>)) -> Option<Vec<u8>> {
        let (key, value) = req;
        match value {
            Some(v) => state.update("default", &key, &v),
            None => state.delete("default", &key),
        }
    }
}

impl CRUDApplication<MapState> for MapApp {
    fn speculatively_execute(
        &self,
        state: &mut impl CRUDState,
        req: (Vec<u8>, Option<Vec<u8>>),
    ) -> Option<Vec<u8>> {
        let (key, value) = req;
        match value {
            Some(v) => state.update("default", &key, &v),
            None => state.delete("default", &key),
        }
    }
}

// ---------------------------------------------------------------------------
// NoopNode — discards all network replies
// ---------------------------------------------------------------------------

pub struct NoopNode;

impl ReplyNode<SMRReply<MapAppData>> for NoopNode {
    fn send(
        &self,
        _rt: RequestType,
        _reply: SMRReply<MapAppData>,
        _target: NodeId,
        _flush: bool,
    ) -> atlas_common::error::Result<()> {
        Ok(())
    }

    fn send_signed(
        &self,
        _rt: RequestType,
        _reply: SMRReply<MapAppData>,
        _target: NodeId,
        _flush: bool,
    ) -> atlas_common::error::Result<()> {
        Ok(())
    }

    fn broadcast(
        &self,
        _rt: RequestType,
        _reply: SMRReply<MapAppData>,
        _targets: impl Iterator<Item = NodeId>,
    ) -> Result<(), Vec<NodeId>> {
        Ok(())
    }

    fn broadcast_signed(
        &self,
        _rt: RequestType,
        _reply: SMRReply<MapAppData>,
        _targets: impl Iterator<Item = NodeId>,
    ) -> Result<(), Vec<NodeId>> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub type KvOp = (Vec<u8>, Option<Vec<u8>>);

/// Build an `UpdateBatch` at sequence number `seq`, containing one entry per op.
pub fn make_batch(seq: u32, ops: &[KvOp]) -> UpdateBatch<KvOp> {
    let mut batch = UpdateBatch::new(SeqNo::from(seq));
    for op in ops {
        batch.add(
            UpdateInfo::new_session_based(NodeId::from(0u32), SeqNo::ZERO, SeqNo::ZERO),
            op.clone(),
        );
    }
    batch
}

/// Timeout for all blocking channel receives in integration tests.
pub const RECV_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn the CRUD preemptive worker and return the send handle together with the
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
    .expect("failed to spawn CRUD preemptive worker");
    (handle, state_tx, checkpoint_rx)
}
