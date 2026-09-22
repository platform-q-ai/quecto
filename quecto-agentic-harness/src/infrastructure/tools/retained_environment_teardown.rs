//! The emptied `retained` environments an owner's exit ends (#2070), over
//! the environment control the slot holds once composition filled it: the
//! records this session created, kept `retained` and emptied — a coordinator
//! that crashed earlier left one — each ended by the same kill
//! `kill_container` runs. An empty slot (a harness without agent-control
//! tools) has nothing to end.

use std::sync::Arc;

use crate::application::subagents::ports::{PortFuture, RetainedEnvironmentTeardown};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus, EnvironmentTarget,
};

use super::agent_cmd_containers::EnvironmentControlSlot;

pub struct SlotRetainedEnvironmentTeardown {
    slot: EnvironmentControlSlot,
}

impl SlotRetainedEnvironmentTeardown {
    pub fn new(slot: EnvironmentControlSlot) -> Arc<Self> {
        Arc::new(Self { slot })
    }
}

/// Whether the owner's exit ends this record: created by this session,
/// kept `retained`, and holding no member.
pub fn ends_on_owner_exit(record: &EnvironmentRecord) -> bool {
    record.origin == EnvironmentOrigin::Created
        && record.status == EnvironmentStatus::Retained
        && record.members.is_empty()
}

impl RetainedEnvironmentTeardown for SlotRetainedEnvironmentTeardown {
    /// The kills run concurrently: the owner's exit waits for the longest
    /// one, not the sum, and each kill script is bounded by the command
    /// adapter (a hung runtime leaves a `cleanup-failed` record, retryable
    /// by an explicit `kill_container`).
    fn end_emptied_retained(&self) -> PortFuture<'_, Vec<(String, Result<(), String>)>> {
        Box::pin(async move {
            let Some(control) = self.slot.get() else {
                return Vec::new();
            };
            let kills = control
                .list
                .execute()
                .into_iter()
                .filter(ends_on_owner_exit)
                .map(|record| {
                    let kill = control.kill.clone();
                    async move {
                        let target = EnvironmentTarget::Ref(record.environment_ref.clone());
                        let outcome = kill
                            .kill_container(&target)
                            .await
                            .map(|_| ())
                            .map_err(|error| error.to_string());
                        (record.environment_ref, outcome)
                    }
                });
            futures::future::join_all(kills).await
        })
    }
}

#[cfg(test)]
#[path = "retained_environment_teardown_tests.rs"]
mod tests;
