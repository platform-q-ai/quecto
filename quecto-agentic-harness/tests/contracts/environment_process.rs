//! Contract for the `EnvironmentProcess` port (#2024 S4d), proven on the
//! production script adapter: liveness is the retained `inspect` argv's
//! own `status` word run against the record's environment id (`running`
//! is live; `dead`/`exited`/`removed`/`stopped` is gone; a script that is
//! missing, fails, times out or says something else is `Unknown` with the
//! reason — never a guess either way); cleanup runs the retained
//! `cleanup` argv once and reports its failure with the script's words.
use std::sync::Arc;

use quecto::application::environments::dto::EnvironmentLiveness;
use quecto::application::environments::ports::EnvironmentProcess;
use quecto::composition::environments::build_environment_process;
use quecto::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};

fn port() -> Arc<dyn EnvironmentProcess> {
    build_environment_process()
}

fn script(dir: &std::path::Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(name);
    std::fs::write(&path, format!("{body}\n")).unwrap();
    vec!["bash".to_string(), path.to_string_lossy().into_owned()]
}

fn record(inspect: Vec<String>, cleanup: Vec<String>) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C7".into(),
        environment_id: "env-seven".into(),
        environment_uuid: "uuid".into(),
        name: None,
        workspace_path: "/state/env-seven/workspace".into(),
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
        origin: EnvironmentOrigin::Restored,
        created_by: String::new(),
        created_at: None,
    }
}

#[test]
fn liveness_is_the_inspect_scripts_status_word_for_the_records_environment_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let seen = dir.path().join("seen");
    let running = script(
        dir.path(),
        "running.sh",
        &format!(
            "printf '%s' \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" > '{}'; printf '{{\"status\":\"running\",\"metadata\":{{\"cause\":\"x\"}}}}'",
            seen.display()
        ),
    );
    assert_eq!(
        port().observe(&record(running, vec![])),
        EnvironmentLiveness::Running
    );
    assert_eq!(std::fs::read_to_string(&seen).unwrap(), "env-seven");
    let dead = script(
        dir.path(),
        "dead.sh",
        r#"printf '{"status":"dead","metadata":{}}'"#,
    );
    assert_eq!(
        port().observe(&record(dead, vec![])),
        EnvironmentLiveness::Gone
    );
}

#[test]
fn a_missing_failing_or_ambiguous_inspect_is_unknown_with_the_reason() {
    let dir = tempfile::TempDir::new().unwrap();
    match port().observe(&record(vec![], vec![])) {
        EnvironmentLiveness::Unknown(reason) => {
            assert!(reason.contains("no retained inspect"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
    let failing = script(
        dir.path(),
        "fail.sh",
        "echo 'podman: connection refused' >&2; exit 1",
    );
    match port().observe(&record(failing, vec![])) {
        EnvironmentLiveness::Unknown(reason) => {
            assert!(reason.contains("connection refused"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
    let odd = script(
        dir.path(),
        "odd.sh",
        r#"printf '{"status":"paused","metadata":{}}'"#,
    );
    match port().observe(&record(odd, vec![])) {
        EnvironmentLiveness::Unknown(reason) => assert!(reason.contains("paused"), "{reason}"),
        other => panic!("{other:?}"),
    }
    let no_status = script(dir.path(), "none.sh", r#"printf '{"metadata":{}}'"#);
    assert!(matches!(
        port().observe(&record(no_status, vec![])),
        EnvironmentLiveness::Unknown(_)
    ));
}

#[test]
fn cleanup_runs_the_retained_argv_once_and_reports_failure_in_the_scripts_words() {
    let dir = tempfile::TempDir::new().unwrap();
    let count = dir.path().join("count");
    let ok = script(
        dir.path(),
        "ok.sh",
        &format!(
            "printf '%s\\n' \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'",
            count.display()
        ),
    );
    port().cleanup(&record(vec![], ok)).unwrap();
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "env-seven\n");
    let failing = script(
        dir.path(),
        "bad.sh",
        "echo 'environment escapes the state root' >&2; exit 1",
    );
    let error = port().cleanup(&record(vec![], failing)).unwrap_err();
    assert!(error.contains("escapes the state root"), "{error}");
    assert!(port().cleanup(&record(vec![], vec![])).is_err());
}
