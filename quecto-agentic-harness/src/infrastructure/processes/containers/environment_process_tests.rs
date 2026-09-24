use super::*;
use crate::application::environments::dto::EnvironmentLiveness;
use crate::domain::environment_registry::{EnvironmentOrigin, EnvironmentStatus};

fn script(dir: &std::path::Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(name);
    std::fs::write(&path, format!("{body}\n")).unwrap();
    vec!["bash".to_string(), path.to_string_lossy().into_owned()]
}

fn record(inspect: Vec<String>, cleanup: Vec<String>) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-1".into(),
        environment_uuid: "uuid-1".into(),
        name: None,
        workspace_path: "/state/env-1/workspace".into(),
        repository: String::new(),
        script_name: "official".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: cleanup,
        retained_inspect_argv: inspect,
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

#[test]
fn status_words_map_to_liveness() {
    assert_eq!(
        liveness_from_inspect_status(Some("running")),
        EnvironmentLiveness::Running
    );
    for dead in ["dead", "exited", "removed", "stopped"] {
        assert_eq!(
            liveness_from_inspect_status(Some(dead)),
            EnvironmentLiveness::Gone
        );
    }
    assert!(
        matches!(liveness_from_inspect_status(Some("weird")), EnvironmentLiveness::Unknown(r) if r.contains("weird"))
    );
    assert!(matches!(
        liveness_from_inspect_status(None),
        EnvironmentLiveness::Unknown(_)
    ));
}

#[test]
fn observe_runs_the_retained_inspect_with_the_environment_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let inspect = script(
        dir.path(),
        "inspect.sh",
        r#"[ "$QUECTO_CONTAINER_ENVIRONMENT_ID" = env-1 ] || exit 3
printf '{"status":"dead","metadata":{"cause":"container-removed"}}'"#,
    );
    let liveness = ScriptEnvironmentProcess.observe(&record(inspect, vec![]));
    assert_eq!(liveness, EnvironmentLiveness::Gone);
}

#[test]
fn a_missing_inspect_or_a_failing_one_is_unknown_with_the_reason() {
    let dir = tempfile::TempDir::new().unwrap();
    let liveness = ScriptEnvironmentProcess.observe(&record(vec![], vec![]));
    assert!(
        matches!(liveness, EnvironmentLiveness::Unknown(r) if r.contains("no retained inspect"))
    );
    let failing = script(
        dir.path(),
        "inspect.sh",
        "echo 'podman is required' >&2; exit 1",
    );
    let liveness = ScriptEnvironmentProcess.observe(&record(failing, vec![]));
    assert!(
        matches!(liveness, EnvironmentLiveness::Unknown(r) if r.contains("podman is required"))
    );
}

#[test]
fn cleanup_reports_success_and_failure_truthfully() {
    let dir = tempfile::TempDir::new().unwrap();
    let marker = dir.path().join("cleaned");
    let ok = script(
        dir.path(),
        "ok.sh",
        &format!("touch '{}'", marker.display()),
    );
    ScriptEnvironmentProcess
        .cleanup(&record(vec![], ok))
        .unwrap();
    assert!(marker.exists());
    let failing = script(
        dir.path(),
        "bad.sh",
        "echo 'state dir escapes root' >&2; exit 1",
    );
    let error = ScriptEnvironmentProcess
        .cleanup(&record(vec![], failing))
        .unwrap_err();
    assert!(error.contains("cleanup failed with status"), "{error}");
    assert!(error.contains("escapes root"), "{error}");
    let error = ScriptEnvironmentProcess
        .cleanup(&record(vec![], vec![]))
        .unwrap_err();
    assert!(error.contains("no retained cleanup argv"), "{error}");
}

#[test]
fn state_on_disk_is_absent_only_when_nothing_exists_under_the_name() {
    // #2134: a stopped record is forgotten only on `Absent`, so every
    // other outcome must keep it.
    let dir = tempfile::TempDir::new().unwrap();
    let process = ScriptEnvironmentProcess;
    let present = dir.path().join("env-1");
    std::fs::create_dir(&present).unwrap();
    assert_eq!(process.state_on_disk(&present), StateOnDisk::Present);
    assert_eq!(
        process.state_on_disk(&dir.path().join("env-2")),
        StateOnDisk::Absent
    );
    let dangling = dir.path().join("env-3");
    std::os::unix::fs::symlink(dir.path().join("nowhere"), &dangling).unwrap();
    assert_eq!(process.state_on_disk(&dangling), StateOnDisk::Present);
    let file = dir.path().join("a-file");
    std::fs::write(&file, "").unwrap();
    assert!(matches!(
        process.state_on_disk(&file.join("env-4")),
        StateOnDisk::Unknown(reason) if reason.contains("could not be examined")
    ));
}
