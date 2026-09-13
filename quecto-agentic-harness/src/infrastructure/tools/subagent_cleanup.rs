//! Exactly-once teardown for registered subagent entries.
//!
//! Slice 2 (#1369): environment children are members of an authoritative
//! session environment record. Tearing a member down removes its membership;
//! the removal that empties a running environment atomically claims the final
//! cleanup, which the environments capability's `FinalizeEnvironmentMember`
//! use case runs exactly once over the script adapters (#1939). Stopped
//! records stay listed; refs are never reused.

use std::sync::Arc;

use super::environment_commands::{HostedStoreObservation, ScriptEnvironmentCommands};
use super::subagent_registry::SubagentRegistry;
use crate::application::environments::use_cases::FinalizeEnvironmentMember;
use crate::domain::environment_registry::EnvironmentRegistry;
use crate::domain::environment_retention::MemberFinalizeMode;

/// One claimed teardown unit: an optional environment membership to remove
/// (with its finalize mode) and an optional per-entry cleanup plan.
struct CleanupJob {
    membership: Option<(EnvironmentRegistry, String, String)>,
    plan: Option<(String, Vec<String>)>,
    mode: MemberFinalizeMode,
}

pub(super) async fn cleanup_registered_once(
    registry: &SubagentRegistry,
    agent_id: &str,
    mode: MemberFinalizeMode,
) {
    let (plan, membership) = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get_mut(agent_id) {
            Some(entry) => (
                entry
                    .cleanup_environment_id
                    .take()
                    .map(|env| (env, std::mem::take(&mut entry.cleanup_argv))),
                take_environment_membership(entry),
            ),
            None => (None, None),
        }
    };
    run_cleanup_jobs(vec![CleanupJob {
        membership: membership.map(|(envs, env_ref)| (envs, env_ref, agent_id.to_string())),
        plan,
        mode,
    }])
    .await;
}

fn take_environment_membership(
    entry: &mut super::subagent_registry::SubagentEntry,
) -> Option<(EnvironmentRegistry, String)> {
    entry
        .environment_registry
        .take()
        .zip(entry.environment_ref.take())
}

fn drain_cleanup_jobs(
    removed: &mut [(String, super::subagent_registry::SubagentEntry)],
    mode: MemberFinalizeMode,
) -> Vec<CleanupJob> {
    removed
        .iter_mut()
        .map(|(id, entry)| CleanupJob {
            plan: entry
                .cleanup_environment_id
                .take()
                .map(|env| (env, std::mem::take(&mut entry.cleanup_argv))),
            membership: take_environment_membership(entry)
                .map(|(envs, env_ref)| (envs, env_ref, id.clone())),
            mode,
        })
        .collect()
}

/// The final-member use case over the production script adapters. The
/// jobs already run on a blocking worker, so the scripts run inline.
fn finalizer(environments: EnvironmentRegistry) -> FinalizeEnvironmentMember {
    FinalizeEnvironmentMember::new(
        environments,
        Arc::new(ScriptEnvironmentCommands::inline()),
        Arc::new(HostedStoreObservation),
    )
}

fn run_cleanup_jobs_sync(jobs: Vec<CleanupJob>) {
    for job in jobs {
        match job.membership {
            Some((environments, env_ref, agent_uuid)) => {
                futures::executor::block_on(finalizer(environments).finalize_member(
                    &env_ref,
                    &agent_uuid,
                    job.plan,
                    job.mode,
                ));
            }
            None => {
                if let Some((env, argv)) = job.plan {
                    use crate::application::environments::ports::EnvironmentProcessCommands;
                    futures::executor::block_on(
                        ScriptEnvironmentCommands::inline().run_retained_cleanup(&env, &argv),
                    );
                }
            }
        }
    }
}

/// Run claimed teardown jobs on a blocking worker so slow container scripts
/// cannot stall the async runtime. A detached `spawn_blocking` task runs to
/// completion even when the awaiting caller is aborted mid-cleanup, so a
/// claimed plan can never be lost.
async fn run_cleanup_jobs(jobs: Vec<CleanupJob>) {
    let has_work = jobs
        .iter()
        .any(|job| job.membership.is_some() || job.plan.is_some());
    if !has_work {
        return;
    }
    let handle = tokio::task::spawn_blocking(move || run_cleanup_jobs_sync(jobs));
    let _ = handle.await;
}

/// Async teardown for cascade-removed entries; use from async contexts so
/// container scripts run on a blocking worker (never on the runtime thread).
pub(super) async fn cleanup_removed_entries_once(
    removed: &mut [(String, super::subagent_registry::SubagentEntry)],
    mode: MemberFinalizeMode,
) {
    run_cleanup_jobs(drain_cleanup_jobs(removed, mode)).await;
}

/// Synchronous per-entry cleanup, kept for the cleanup tests that exercise
/// the exactly-once claim without a runtime.
#[cfg(test)]
pub(super) fn cleanup_removed_entries_sync(
    removed: &mut [(String, super::subagent_registry::SubagentEntry)],
) {
    // Process shutdown is parent-initiated, not a post-mortem: skip inspect.
    run_cleanup_jobs_sync(drain_cleanup_jobs(removed, MemberFinalizeMode::ParentKill));
}
