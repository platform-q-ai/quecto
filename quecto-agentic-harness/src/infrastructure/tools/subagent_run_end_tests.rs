//! #2206 end to end through the production fleet teardown and member
//! finalizer, over real (logging) scripts in a temp dir: a one-shot
//! parent's run end cleans a plain container child's box up and forgets
//! its record; a harness shutdown — which may be a crash — keeps the
//! ordinary retained kill and the record; a selected kill of the child is
//! the owner's word too.

use crate::infrastructure::test_support::executable::write_executable;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::subagent_registry::{SubagentEntry, SubagentRegistry};
use crate::application::subagents::dto::{
    FleetTeardownAuthority, TerminateAllDelegatedAgentsRequest,
};
use crate::application::subagents::ports::{TeardownCompensation, TerminationCause};
use crate::domain::agents::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration, ShutdownReason,
};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
    mint_environment_uuid,
};

/// A script that appends `<op> <environment id>` to `log`.
fn logging_script(dir: &Path, op: &str, log: &Path) -> String {
    let script = dir.join(format!("{op}.sh"));
    write_executable(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"{op} $QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\n",
            log.display()
        ),
    );
    script.to_string_lossy().to_string()
}

struct Rig {
    _temp: tempfile::TempDir,
    log: PathBuf,
    environments: EnvironmentRegistry,
    env_ref: String,
    registry: SubagentRegistry,
}

/// One plain container child (no coordination store under its workspace)
/// this harness launched, the only member of the environment it created.
fn rig() -> Rig {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("scripts.log");
    // A plain container: its workspace resolves and holds no board.
    std::fs::create_dir_all(temp.path().join("workspace")).unwrap();
    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref().unwrap();
    environments.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-plain".to_string(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: temp.path().join("workspace"),
        repository: String::new(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![logging_script(temp.path(), "kill", &log)],
        retained_cleanup_argv: vec![logging_script(temp.path(), "cleanup", &log)],
        retained_inspect_argv: vec![],
        members: vec!["child".to_string()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    });
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::with_identity(
        crate::domain::ids::AgentUuid::new("child"),
        "child".into(),
        temp.path().join("never.sock"),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(1));
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    registry.lock().unwrap().insert("child".to_string(), entry);
    Rig {
        _temp: temp,
        log,
        environments,
        env_ref,
        registry,
    }
}

impl Rig {
    fn scripts(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    async fn fleet(&self, authority: FleetTeardownAuthority) {
        let fleet = crate::composition::subagent_teardown::build_fleet_teardown(
            crate::composition::subagent_teardown::FleetTeardownWiring {
                owner: crate::domain::ids::AgentUuid::new("root"),
                registry: self.registry.clone(),
                broadcast_tx: None,
                notify_tx: None,
                harness_lifecycle: super::harness_lifecycle::new_shared_harness_lifecycle(),
            },
        );
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            fleet.execute(TerminateAllDelegatedAgentsRequest {
                reason: ShutdownReason::ParentShutdown,
                authority,
            }),
        )
        .await
        .expect("bounded")
        .expect("the fleet run completed");
        assert_eq!(outcome.removed_count(), 1, "{outcome:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_one_shot_run_end_cleans_a_plain_child_box_up_and_forgets_it() {
    let rig = rig();
    rig.fleet(FleetTeardownAuthority::RunEnd).await;
    assert_eq!(rig.scripts(), vec!["cleanup env-plain".to_string()]);
    assert!(
        rig.environments.get(&rig.env_ref).is_none(),
        "the record is forgotten"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_harness_shutdown_keeps_the_retained_kill_and_the_record() {
    let rig = rig();
    rig.fleet(FleetTeardownAuthority::Harness).await;
    assert_eq!(rig.scripts(), vec!["kill env-plain".to_string()]);
    assert_eq!(
        rig.environments.get(&rig.env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_selected_kill_of_a_plain_child_is_the_owners_word() {
    let rig = rig();
    let agents = super::subagent_teardown_registry::RegistryDelegatedAgents::new(
        rig.registry.clone(),
        None,
        None,
        crate::composition::environments::build_member_finalizer,
    );
    let child = DelegatedAgentIdentity::new(
        crate::domain::ids::AgentUuid::new("child"),
        LaunchGeneration::new(1),
    );
    agents
        .compensate(&child, TerminationCause::SelectedTermination)
        .await;
    assert_eq!(rig.scripts(), vec!["cleanup env-plain".to_string()]);
    assert!(rig.environments.get(&rig.env_ref).is_none());
}

/// The exit of a child the run end already claimed stopping (the reaper or
/// the monitor observing it first) is that run end's, never a post-mortem:
/// its plain container still ends for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exit_observed_for_a_run_end_claim_is_the_run_ends() {
    use crate::application::subagents::ports::{
        DelegatedAgentRegistry, ExitObservation, TerminalClaim,
    };
    let rig = rig();
    let agents = super::subagent_teardown_registry::RegistryDelegatedAgents::new(
        rig.registry.clone(),
        None,
        None,
        crate::composition::environments::build_member_finalizer,
    );
    let child = DelegatedAgentIdentity::new(
        crate::domain::ids::AgentUuid::new("child"),
        LaunchGeneration::new(1),
    );
    agents
        .claim_stopping(&child, TerminationCause::RunEnd)
        .unwrap();
    assert_eq!(agents.claim_terminal(&child), TerminalClaim::Claimed);
    agents
        .compensate(
            &child,
            TerminationCause::Exit(ExitObservation::ProcessExited),
        )
        .await;
    assert_eq!(rig.scripts(), vec!["cleanup env-plain".to_string()]);
    assert!(rig.environments.get(&rig.env_ref).is_none());
}
