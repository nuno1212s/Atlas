//! Shared test fixtures for `single_thread_double_state` tests.
//!
//! This module is compiled only under `#[cfg(test)]`. It provides the minimal
//! counter application types and the `make_batch` helper that are used by the
//! unit tests in `confirmed_requests`, `preemptive_requests`, and the
//! integration tests in `tests`.

use atlas_common::node_id::NodeId;
use atlas_common::ordering::SeqNo;
use atlas_core::execution::requests::{IncrementableUpdateBatch, UpdateBatch, UpdateInfo};
use atlas_smr_application::app::Application;
use atlas_smr_application::serialize::ApplicationData;

// ---------------------------------------------------------------------------
// TestData — minimal ApplicationData with u32 request and reply
// ---------------------------------------------------------------------------

/// Minimal `ApplicationData` whose request and reply are both `u32`.
///
/// All serialisation methods are no-ops (test code never exercises them over
/// the wire).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct TestData;

impl ApplicationData for TestData {
    type Request = u32;
    type Reply = u32;

    fn serialize_request<W>(_: W, _: &u32) -> atlas_common::error::Result<()>
    where
        W: std::io::Write,
    {
        Ok(())
    }

    fn deserialize_request<R>(_: R) -> atlas_common::error::Result<u32>
    where
        R: std::io::Read,
    {
        Ok(0)
    }

    fn serialize_reply<W>(_: W, _: &u32) -> atlas_common::error::Result<()>
    where
        W: std::io::Write,
    {
        Ok(())
    }

    fn deserialize_reply<R>(_: R) -> atlas_common::error::Result<u32>
    where
        R: std::io::Read,
    {
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// TestApp — simple counter application over a plain u32 state
// ---------------------------------------------------------------------------

/// Simple counter application.
///
/// State = running total (`u32`), request = value to add, reply = new total.
pub struct TestApp;

impl Application<u32> for TestApp {
    type AppData = TestData;

    fn initial_state() -> atlas_common::error::Result<u32> {
        Ok(0)
    }

    fn unordered_execution(&self, state: &u32, _req: u32) -> u32 {
        *state
    }

    fn update(&self, state: &mut u32, req: u32) -> u32 {
        *state += req;
        *state
    }
}

// ---------------------------------------------------------------------------
// make_batch — UpdateBatch<u32> factory
// ---------------------------------------------------------------------------

/// Build an `UpdateBatch<u32>` at sequence number `seq`, containing one entry
/// per element of `ops`.
pub fn make_batch(seq: u32, ops: &[u32]) -> UpdateBatch<u32> {
    let mut batch = UpdateBatch::new(SeqNo::from(seq));
    for &op in ops {
        batch.add(
            UpdateInfo::new_session_based(NodeId::from(0u32), SeqNo::ZERO, SeqNo::from(op)),
            op,
        );
    }
    batch
}

