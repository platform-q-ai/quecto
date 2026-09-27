//! #2206: `quecto container kill <ref>` removes a `stopped` environment's
//! leftovers through its retained cleanup and forgets the record — only
//! once its own inspect says the container is gone.

use super::tests::{run, script};
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};
use crate::infrastructure::persistence::environment_registry_store::FileEnvironmentRegistryStore;
use crate::interface::cli::CliContext;

/// A base dir whose registry records two `stopped` environments that left
/// their state directories behind: `C2`, whose inspect says the container
/// exited, and `C4`, whose inspect says it still runs (another session's
/// box). Their cleanup logs itself and removes the state directory.
fn stopped_leftovers() -> (tempfile::TempDir, CliContext, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let base = dir.path().join("base");
    let state = dir.path().join("state");
    for id in ["env-two", "env-four"] {
        std::fs::create_dir_all(state.join(id).join("workspace")).unwrap();
    }
    let exited = script(
        dir.path(),
        "exited.sh",
        r#"printf '{"status":"exited","metadata":{}}'"#,
    );
    let running = script(
        dir.path(),
        "running.sh",
        r#"printf '{"status":"running","metadata":{}}'"#,
    );
    let log = dir.path().join("scripts.log");
    let cleanup = script(
        dir.path(),
        "cleanup.sh",
        &format!(
            "echo \"cleanup $QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'; rm -rf '{}'/\"$QUECTO_CONTAINER_ENVIRONMENT_ID\"",
            log.display(),
            state.display()
        ),
    );
    let kill = script(
        dir.path(),
        "kill.sh",
        &format!(
            "echo \"kill $QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'",
            log.display()
        ),
    );
    std::fs::create_dir_all(&base).unwrap();
    let store = FileEnvironmentRegistryStore::for_base_dir(&base);
    for (reference, id, inspect) in [("C2", "env-two", &exited), ("C4", "env-four", &running)] {
        store
            .record(&EnvironmentRecord {
                environment_ref: reference.to_string(),
                environment_id: id.to_string(),
                environment_uuid: format!("uuid-{reference}"),
                name: Some(format!("name-{reference}")),
                workspace_path: state.join(id).join("workspace"),
                repository: "https://example.test/repo".into(),
                script_name: "official".into(),
                retained_exec_argv: vec!["exec".into()],
                retained_kill_argv: kill.clone(),
                retained_cleanup_argv: cleanup.clone(),
                retained_inspect_argv: inspect.clone(),
                members: vec![],
                status: EnvironmentStatus::Stopped,
                metadata: serde_json::json!({}),
                last_error: None,
                origin: EnvironmentOrigin::Created,
                created_by: "cli:qa".into(),
                created_at: Some(0),
            })
            .unwrap();
    }
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let ctx = CliContext {
        base_dir: Some(base),
        cwd: Some(cwd),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        container_inventory: Some(crate::composition::environments::build_container_inventory),
        ..Default::default()
    };
    (dir, ctx, log)
}

fn on_file(ctx: &CliContext) -> Vec<String> {
    FileEnvironmentRegistryStore::for_base_dir(&ctx.base_dir())
        .load()
        .unwrap()
        .into_iter()
        .map(|record| format!("{} {}", record.environment_ref, record.status_label()))
        .collect()
}

#[test]
fn kill_removes_a_stopped_environment_whose_container_exited_and_forgets_it() {
    let (dir, ctx, log) = stopped_leftovers();
    let output = run(&["container", "kill", "C2"], &ctx);
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert_eq!(
        output.stdout,
        "removed stopped C2 (name-C2): its container and state directory are gone and its record is forgotten\n"
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "cleanup env-two\n");
    assert!(
        !dir.path().join("state/env-two").exists(),
        "leftovers removed"
    );
    assert_eq!(on_file(&ctx), ["C4 stopped"], "C2 forgotten on file");
}

#[test]
fn kill_refuses_a_stopped_environment_whose_container_still_runs() {
    let (dir, ctx, log) = stopped_leftovers();
    let output = run(&["container", "kill", "C4"], &ctx);
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(
        output
            .stderr
            .contains("environment C4 is stopped in the registry, but its container is running"),
        "{output:?}"
    );
    assert!(!log.exists(), "no script ran");
    assert!(dir.path().join("state/env-four/workspace").exists());
    assert_eq!(on_file(&ctx), ["C2 stopped", "C4 stopped"], "untouched");
}
