//! #2070: how the fleet going ends — or does not end — a swarm's environment,
//! through the production compensation port against a real coordination
//! store and a real (logging) retained kill script.

use super::*;
use crate::application::subagents::ports::TerminationCause;

const OWNER: TerminationCause = TerminationCause::OwnerTeardown;
const HARNESS: TerminationCause = TerminationCause::FleetTeardown;

/// The production fleet-teardown path (#1938): `TerminateAllDelegatedAgents`
/// claims the row and compensates it through the registry port, which maps
/// the cause to the finalize mode. Against a real store and kill script.
async fn fleet_end(
    hosted: fn(&std::path::Path) -> SwarmContext,
    cause: TerminationCause,
    compensated_as: TerminationCause,
) -> (EnvironmentRecord, bool) {
    use crate::application::subagents::ports::{DelegatedAgentRegistry, TeardownCompensation};
    use crate::domain::subagent_teardown::{DelegatedAgentIdentity, LaunchGeneration};
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let _context = hosted(&checkout);
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);
    let uuid = {
        let mut rows = registry.lock().unwrap();
        let row = rows.get_mut("coordinator-1").unwrap();
        row.launch_generation = Some(LaunchGeneration::new(1));
        row.agent_uuid.clone()
    };
    let port =
        crate::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents::new(
            registry.clone(),
            None,
            None,
            crate::composition::environments::build_member_finalizer,
        );
    let id = DelegatedAgentIdentity::new(uuid.as_str(), LaunchGeneration::new(1));
    port.claim_stopping(&id, cause).unwrap();
    port.claim_terminal(&id);
    port.compensate(&id, compensated_as).await;
    (environments.get(&env_ref).unwrap(), log.exists())
}

#[tokio::test]
async fn an_owners_teardown_removes_a_running_swarms_environment() {
    // #2070: delete-all or a session transition ends the owner's swarms.
    let (record, killed) = fleet_end(create_running_swarm, OWNER, OWNER).await;
    assert!(killed, "the retained kill ran");
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert!(record.metadata.get("retained").is_none(), "{record:?}");
}

#[tokio::test]
async fn a_child_that_exits_by_itself_during_the_teardown_follows_the_claimed_intent() {
    // The usual production order: the row is claimed stopping, the child
    // exits gracefully, and the reaper compensates it as an `Exit`. The
    // intent the claim stored decides, not the exit (#2070).
    let exited = TerminationCause::Exit(
        crate::application::subagents::ports::ExitObservation::ProcessExited,
    );
    let (record, killed) = fleet_end(create_running_swarm, OWNER, exited).await;
    assert!(killed, "claimed by the owner's teardown: the box goes");
    assert_eq!(record.status, EnvironmentStatus::Stopped);

    let (record, killed) = fleet_end(create_running_swarm, HARNESS, exited).await;
    assert!(!killed, "claimed by a harness shutdown: the box stays");
    assert_eq!(record.status, EnvironmentStatus::Retained);
}

#[tokio::test]
async fn a_harness_shutdown_keeps_a_running_swarms_environment() {
    // A shutdown can be a crash (a signal, the last client gone, a lost
    // parent): the run has not ended, so the box stays resumable (#2070).
    let (record, killed) = fleet_end(create_running_swarm, HARNESS, HARNESS).await;
    assert!(!killed, "a harness shutdown keeps a swarm's box");
    assert_eq!(record.status, EnvironmentStatus::Retained);
    let reason = record.metadata["retained"].as_str().unwrap();
    assert!(
        reason.starts_with("coordinator killed by supervisor; run running"),
        "{reason}"
    );
}

#[tokio::test]
async fn a_harness_shutdown_still_kills_an_ordinary_environment() {
    let (record, killed) = fleet_end(bootstrap_placeholder, HARNESS, HARNESS).await;
    assert!(killed);
    assert_eq!(record.status, EnvironmentStatus::Stopped);
}
