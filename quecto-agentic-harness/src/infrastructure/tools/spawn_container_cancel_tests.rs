//! #2173: a launch dropped before registration rolls back, including one
//! dropped while its container create is still running.
use super::tests::{base_config, test_selection, test_supervisor};
use super::*;
use std::time::Duration;
use tempfile::TempDir;

/// A config whose create waits (up to 5 s) for `<dir>/gate` to exist (and, sent
/// SIGTERM, writes `<dir>/stopped`), and whose cleanup writes the
/// environment id it was given to `<dir>/cleaned`.
fn write_config(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("config.json");
    let create = format!(
        r#"trap 'echo stopped > {stopped}; exit 143' TERM; n=0; while [ ! -e {gate} ] && [ $n -lt 250 ]; do sleep 0.02; n=$((n+1)); done; printf '{{"environment_id":"env-1","workspace_path":"/tmp/ws","socket_path":"/tmp/s.sock","metadata":{{}}}}'"#,
        gate = dir.join("gate").display(),
        stopped = dir.join("stopped").display()
    );
    let cleanup = format!(
        r#"printf %s "$QUECTO_CONTAINER_ENVIRONMENT_ID" > {}"#,
        dir.join("cleaned").display()
    );
    std::fs::write(
        &path,
        serde_json::json!({"container_configs": {"box": {
            "default": true,
            "create": ["/bin/sh", "-c", create],
            "cleanup": ["/bin/sh", "-c", cleanup],
        }}})
        .to_string(),
    )
    .unwrap();
    path
}

async fn launch(dir: &Path, registry: &EnvironmentRegistry) -> Result<PreparedChild, DomainError> {
    let mut config = base_config(ContainerSelection::New {
        container_config: Some("box".into()),
        name: None,
    });
    config.config_path = Some(write_config(dir));
    spawn_prepared_child(
        &config,
        &ChildCommand {
            swarm_context: None,
            swarm_member: false,
            supervisor: &test_supervisor(),
            binary: Path::new("true"),
            cli_args: &[],
            base_dir: dir,
            admission_dir: None,
            context_mode: crate::domain::conversation::ContextMode::Default,
        },
        registry,
        Some(&test_selection(dir)),
    )
    .await
}

fn open_gate(dir: &Path) {
    std::fs::write(dir.join("gate"), "").unwrap();
}

fn cleaned(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("cleaned")).ok()
}

async fn rollbacks_settle() {
    assert!(super::super::launch_rollbacks::settled(Duration::from_secs(30)).await);
}

/// The create a dropped spawn was running is stopped, not waited for: its
/// script gets SIGTERM to remove what it made, and nothing is recorded.
#[tokio::test]
async fn a_spawn_dropped_during_its_create_stops_the_create() {
    let dir = TempDir::new().unwrap();
    let registry = EnvironmentRegistry::new();
    let stopped =
        tokio::time::timeout(Duration::from_millis(100), launch(dir.path(), &registry)).await;
    assert!(stopped.is_err(), "the create waits for its gate");
    rollbacks_settle().await;
    assert_eq!(
        std::fs::read_to_string(dir.path().join("stopped"))
            .ok()
            .as_deref(),
        Some("stopped\n"),
        "the create was told to stop"
    );
    assert_eq!(cleaned(dir.path()), None, "nothing was committed to clean");
    assert!(registry.get("C1").is_none());
}

#[tokio::test]
async fn a_prepared_launch_dropped_before_registration_rolls_back() {
    let dir = TempDir::new().unwrap();
    let registry = EnvironmentRegistry::new();
    open_gate(dir.path());
    let prepared = launch(dir.path(), &registry).await.unwrap();
    assert!(registry.get("C1").is_some());
    drop(prepared);
    rollbacks_settle().await;
    assert_eq!(cleaned(dir.path()).as_deref(), Some("env-1"));
    assert!(registry.get("C1").is_none());
}

#[tokio::test]
async fn a_handed_over_launch_is_left_to_its_registry_entry() {
    let dir = TempDir::new().unwrap();
    let registry = EnvironmentRegistry::new();
    open_gate(dir.path());
    let mut prepared = launch(dir.path(), &registry).await.unwrap();
    prepared.hand_over();
    drop(prepared);
    rollbacks_settle().await;
    assert_eq!(cleaned(dir.path()), None);
    assert!(registry.get("C1").is_some());
}

/// Review finding: a rollback cancelled while it asks the child to shut
/// down keeps the handle, so the drop guard still ends the child.
#[tokio::test]
async fn a_rollback_cancelled_while_ending_the_child_is_finished_by_the_guard() {
    let dir = TempDir::new().unwrap();
    // Accepts (into its backlog) and never answers: the ask waits.
    let silent = dir.path().join("silent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&silent).unwrap();
    let mut sleep = tokio::process::Command::new("sleep");
    sleep.arg("30");
    let mut prepared = PreparedChild::new_for_test(Some(sleep), None, None).await;
    let supervisor = Arc::clone(&prepared.supervisor);
    let handle = prepared.owned_child.expect("a child");
    let stopped = tokio::time::timeout(
        Duration::from_millis(100),
        prepared.rollback_once_via(Some(&silent)),
    )
    .await;
    assert!(stopped.is_err(), "the unanswered ask outlasts the wait");
    assert!(prepared.owned_child == Some(handle), "the handle is kept");
    drop(prepared);
    rollbacks_settle().await;
    // Ended (or already retired): either way the wait returns at once,
    // where a live child would hold it for 30 s.
    assert!(
        tokio::time::timeout(Duration::from_secs(2), supervisor.wait_exit(handle))
            .await
            .is_ok(),
        "the guard's rollback ended the child"
    );
}

/// Review finding: a rollback cancelled during its cleanup script does not
/// start the script a second time.
#[tokio::test]
async fn a_rollback_cancelled_during_cleanup_does_not_run_it_twice() {
    let dir = TempDir::new().unwrap();
    let registry = EnvironmentRegistry::new();
    registry.commit(super::tests::test_record("C1", "env-1"));
    let log = dir.path().join("cleanups");
    let mut prepared = PreparedChild::new_for_test(None, Some("C1".into()), None).await;
    prepared.environments = Some(registry.clone());
    prepared.cleanup_environment_id = Some("env-1".into());
    prepared.cleanup_argv = vec![
        "/bin/sh".into(),
        "-c".into(),
        format!("echo run >> {}; sleep 0.5", log.display()),
    ];
    let stopped =
        tokio::time::timeout(Duration::from_millis(200), prepared.rollback_once_via(None)).await;
    assert!(stopped.is_err(), "the cleanup outlasts the wait");
    drop(prepared);
    rollbacks_settle().await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "run\n");
    assert!(registry.get("C1").is_none(), "the guard removed the record");
}
