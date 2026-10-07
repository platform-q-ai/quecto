//! #2206: a one-shot run's orderly end settles the children it launched on
//! the run-end authority; dropping it (a panic unwinding, an early return)
//! settles nothing.

use crate::infrastructure::test_support::executable::write_executable;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::super::agent::AgentOutput;
use super::{RunEnd, RunEndFleetBuilder, waiting_for};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
    mint_environment_uuid,
};
use crate::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};

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

/// A registry holding one launched plain container child, the only member
/// of the environment the run created.
fn launched_container_child(
    dir: &Path,
    log: &Path,
) -> (SubagentRegistry, EnvironmentRegistry, String) {
    // A plain container: its workspace resolves and holds no board.
    std::fs::create_dir_all(dir.join("workspace")).unwrap();
    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref().unwrap();
    environments.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-run".to_string(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: dir.join("workspace"),
        repository: String::new(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![logging_script(dir, "kill", log)],
        retained_cleanup_argv: vec![logging_script(dir, "cleanup", log)],
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
        dir.join("never.sock"),
        0,
    );
    entry.launch_generation =
        Some(crate::domain::agents::subagent_teardown::LaunchGeneration::new(1));
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    registry.lock().unwrap().insert("child".to_string(), entry);
    (registry, environments, env_ref)
}

const BUILD: RunEndFleetBuilder = crate::composition::subagent_teardown::build_run_end_fleet;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn an_orderly_run_end_cleans_its_plain_container_child_up_and_forgets_it() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("scripts.log");
    let (registry, environments, env_ref) = launched_container_child(temp.path(), &log);
    let rt = runtime();

    let (mut stdout, mut stderr) = ("the answer\n".to_string(), String::new());
    let code = RunEnd::compose(
        Some(BUILD),
        Some(registry.clone()),
        Some(new_shared_harness_lifecycle()),
        false,
    )
    .after(
        &rt,
        &mut AgentOutput {
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
        2,
    );

    assert_eq!(code, 2, "the run's own exit code is handed back");
    assert_eq!(stdout, "the answer\n", "a captured run keeps its buffer");

    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "cleanup env-run"
    );
    assert!(environments.get(&env_ref).is_none(), "record forgotten");
    assert!(
        registry.lock().unwrap().is_empty(),
        "the row left the roster"
    );
}

#[test]
fn a_run_end_that_is_dropped_unsettled_ends_nothing() {
    // A panic unwinding or an early return drops the run end: that is no
    // orderly end, so the box and its record stay for the parent-loss path
    // and `gc`.
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("scripts.log");
    let (registry, environments, env_ref) = launched_container_child(temp.path(), &log);
    let rt = runtime();

    drop(RunEnd::compose(
        Some(BUILD),
        Some(registry.clone()),
        Some(new_shared_harness_lifecycle()),
        true,
    ));
    drop(rt);

    assert!(!log.exists(), "no script ran");
    assert_eq!(
        environments.get(&env_ref).unwrap().status,
        EnvironmentStatus::Running
    );
    assert_eq!(registry.lock().unwrap().len(), 1);
}

#[test]
fn nothing_is_settled_without_a_builder_a_registry_or_a_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("scripts.log");
    let (registry, environments, env_ref) = launched_container_child(temp.path(), &log);
    let rt = runtime();
    for run_end in [
        RunEnd::none(),
        RunEnd::compose(
            None,
            Some(registry.clone()),
            Some(new_shared_harness_lifecycle()),
            false,
        ),
        RunEnd::compose(
            Some(BUILD),
            None,
            Some(new_shared_harness_lifecycle()),
            false,
        ),
        RunEnd::compose(Some(BUILD), Some(registry.clone()), None, false),
    ] {
        run_end.settle(&rt);
    }
    assert!(!log.exists());
    assert_eq!(
        environments.get(&env_ref).unwrap().status,
        EnvironmentStatus::Running
    );
}

#[test]
fn a_live_run_writes_its_answer_out_before_it_settles() {
    // `quecto agent -m`: the answer reaches the terminal first, so nothing
    // waits on the children; the buffers the command prints later are empty.
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("scripts.log");
    let (registry, _environments, _env_ref) = launched_container_child(temp.path(), &log);
    let rt = runtime();
    let (mut stdout, mut stderr) = ("the answer\n".to_string(), "a note\n".to_string());
    let code = RunEnd::compose(
        Some(BUILD),
        Some(registry),
        Some(new_shared_harness_lifecycle()),
        true,
    )
    .after(
        &rt,
        &mut AgentOutput {
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
        0,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.is_empty() && stderr.is_empty(),
        "written out already"
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "cleanup env-run"
    );
}

#[test]
fn the_run_end_names_the_children_it_is_waiting_for() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("scripts.log");
    let (registry, _environments, _env_ref) = launched_container_child(temp.path(), &log);
    assert_eq!(waiting_for(&registry), ["child".to_string()]);
}
