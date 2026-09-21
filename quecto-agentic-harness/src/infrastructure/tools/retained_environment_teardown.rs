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
    fn end_emptied_retained(&self) -> PortFuture<'_, Vec<(String, Result<(), String>)>> {
        Box::pin(async move {
            let Some(control) = self.slot.get() else {
                return Vec::new();
            };
            let mut ended = Vec::new();
            for record in control.list.execute() {
                if !ends_on_owner_exit(&record) {
                    continue;
                }
                let target = EnvironmentTarget::Ref(record.environment_ref.clone());
                let outcome = control
                    .kill
                    .kill_container(&target)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                ended.push((record.environment_ref, outcome));
            }
            ended
        })
    }
}

#[cfg(test)]
#[path = "retained_environment_teardown_tests.rs"]
mod tests;
