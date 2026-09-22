//! The emptied `retained` environments an owner's exit ends (#2070), over
//! the environment control the slot holds once composition filled it: the
//! records this session created, kept `retained` and emptied — a coordinator
//! that crashed earlier left one — each ended by the same kill
//! `kill_container` runs. An empty slot (a harness without agent-control
//! tools) has nothing to end.

use std::sync::Arc;
use std::time::Duration;

use crate::application::subagents::ports::{PortFuture, RetainedEnvironmentTeardown};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus, EnvironmentTarget,
};

use super::agent_cmd_containers::EnvironmentControlSlot;

/// Bound on one environment's kill. The owner is waiting for the harness
/// to exit on a budget derived from the fleet, not from these boxes: a
/// runtime that hangs on `rm` is reported and left, not waited for. The
/// kill job runs on; its record stays `killing` and an explicit
/// `kill_container` retries it.
pub const KILL_BOUND: Duration = Duration::from_secs(10);

pub struct SlotRetainedEnvironmentTeardown {
    slot: EnvironmentControlSlot,
    kill_bound: Duration,
}

impl SlotRetainedEnvironmentTeardown {
    pub fn new(slot: EnvironmentControlSlot) -> Arc<Self> {
        Arc::new(Self {
            slot,
            kill_bound: KILL_BOUND,
        })
    }

    #[cfg(test)]
    pub fn with_kill_bound(slot: EnvironmentControlSlot, kill_bound: Duration) -> Arc<Self> {
        Arc::new(Self { slot, kill_bound })
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
                let outcome = match tokio::time::timeout(
                    self.kill_bound,
                    control.kill.kill_container(&target),
                )
                .await
                {
                    Ok(killed) => killed.map(|_| ()).map_err(|error| error.to_string()),
                    Err(_) => Err(format!(
                        "kill did not finish within {}s; the record stays killing, retry kill_container",
                        self.kill_bound.as_secs()
                    )),
                };
                ended.push((record.environment_ref, outcome));
            }
            ended
        })
    }
}

#[cfg(test)]
#[path = "retained_environment_teardown_tests.rs"]
mod tests;
