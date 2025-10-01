use crate::timeouts::{TimeOutable, Timeout, TimeoutID, TimeoutIdentification, TimeoutsHandle};
use atlas_common::node_id::NodeId;
use getset::{CopyGetters, Getters};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::instrument;

/// A mod that is compatible with the timeouts layer
/// It should present a name for the mod and a function to handle
/// any timeouts that it receives
pub trait TimeoutableMod<TR> {
    /// The name of this module, as an Arc<str>
    fn mod_name() -> Arc<str>;

    /// Handle a timeout received from the timeouts layer
    fn handle_timeout(&mut self, timeout: Vec<ModTimeout>) -> atlas_common::error::Result<TR>;
}

#[derive(Clone)]
pub struct TimeoutModHandle {
    mod_name: Arc<str>,
    timeout_handle: TimeoutsHandle,
}

#[derive(Getters, CopyGetters, Debug)]
pub struct ModTimeout {
    #[get = "pub"]
    id: TimeoutID,
    #[get_copy = "pub"]
    timeout_count: usize,
    #[get_copy = "pub"]
    timeout_time: SystemTime,
    extra_info: Option<Box<dyn TimeOutable>>,
}

impl TimeoutModHandle {
    #[instrument(skip(self), level = "trace", fields(mod_name = &*self.mod_name))]
    pub fn request_timeout(
        &self,
        timeout_id: TimeoutID,
        extra_info: Option<Box<dyn TimeOutable>>,
        duration: Duration,
        needed_acks: usize,
        cumulative: bool,
    ) -> atlas_common::error::Result<()> {
        self.timeout_handle.request_timeout(
            TimeoutIdentification::new_from_id(self.mod_name.clone(), timeout_id),
            extra_info,
            duration,
            needed_acks,
            cumulative,
        )
    }

    #[instrument(skip(self, timeout_id), level = "trace", fields(mod_name = &*self.mod_name))]
    pub fn request_timeouts(
        &self,
        timeout_id: Vec<(TimeoutID, Option<Box<dyn TimeOutable>>)>,
        duration: Duration,
        needed_acks: usize,
        cumulative: bool,
    ) -> atlas_common::error::Result<()> {
        let mapped_timeouts = timeout_id
            .into_iter()
            .map(|(id, extra_info)| {
                (
                    TimeoutIdentification::new_from_id(self.mod_name.clone(), id),
                    extra_info,
                )
            })
            .collect();

        self.timeout_handle
            .request_timeouts(mapped_timeouts, duration, needed_acks, cumulative)
    }

    #[instrument(skip(self), level = "trace", fields(mod_name = &*self.mod_name))]
    pub fn ack_received(
        &self,
        timeout_id: TimeoutID,
        from: NodeId,
    ) -> atlas_common::error::Result<()> {
        self.timeout_handle.ack_received(
            TimeoutIdentification::new_from_id(self.mod_name.clone(), timeout_id),
            from,
        )
    }

    #[instrument(skip_all, level = "trace", fields(mod_name = &*self.mod_name))]
    pub fn acks_received(&self, acks: Vec<(TimeoutID, NodeId)>) -> atlas_common::error::Result<()> {
        let mapped_acks = acks
            .into_iter()
            .map(|(id, from)| {
                (
                    TimeoutIdentification::new_from_id(self.mod_name.clone(), id),
                    from,
                )
            })
            .collect();

        self.timeout_handle.acks_received(mapped_acks)
    }

    #[instrument(skip(self), level = "trace", fields(mod_name = &*self.mod_name))]
    pub fn cancel_timeout(&self, timeout_id: TimeoutID) -> atlas_common::error::Result<()> {
        self.timeout_handle
            .cancel_timeout(TimeoutIdentification::new_from_id(
                self.mod_name.clone(),
                timeout_id,
            ))
    }

    #[instrument(skip(self), level = "trace", fields(mod_name = &*self.mod_name))]
    pub fn cancel_all_timeouts(&self) -> atlas_common::error::Result<()> {
        self.timeout_handle
            .cancel_all_timeouts_for_mod(self.mod_name.clone())
    }

    #[instrument(skip(self), level = "trace", fields(mod_name = &*self.mod_name))]
    pub fn cancel_timeouts(
        &self,
        timeouts_to_cancel: Vec<TimeoutID>,
    ) -> atlas_common::error::Result<()> {
        self.timeout_handle.cancel_timeouts(
            timeouts_to_cancel
                .into_iter()
                .map(|id| TimeoutIdentification::new_from_id(self.mod_name.clone(), id))
                .collect(),
        )
    }

    #[instrument(skip(self), level = "trace", fields(mod_name = &*self.mod_name))]
    pub fn reset_all_timeouts(&self) -> atlas_common::error::Result<()> {
        self.timeout_handle
            .reset_all_timeouts_for_mod(self.mod_name.clone())
    }
}

impl TimeoutModHandle {
    pub(super) fn from_timeout_mod<M, R>(timeout: TimeoutsHandle) -> Self
    where
        M: TimeoutableMod<R>,
    {
        Self {
            mod_name: M::mod_name(),
            timeout_handle: timeout,
        }
    }

    pub(super) fn from_name(name: Arc<str>, timeout: TimeoutsHandle) -> Self {
        Self {
            mod_name: name,
            timeout_handle: timeout,
        }
    }
}

impl ModTimeout {
    pub fn extra_info(&self) -> Option<&dyn TimeOutable> {
        self.extra_info.as_deref()
    }
}

impl From<Timeout> for ModTimeout {
    fn from(value: Timeout) -> Self {
        Self {
            id: value.id.timeout_id,
            timeout_count: value.timeout_count,
            timeout_time: value.timeout_time,
            extra_info: value.extra_info,
        }
    }
}

impl Clone for ModTimeout {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            timeout_count: self.timeout_count,
            timeout_time: self.timeout_time,
            extra_info: None,
        }
    }
}
