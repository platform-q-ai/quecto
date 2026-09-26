//! #2173: a launch dropped before registration rolls back, including one
//! dropped while its container create is still running.
use super::tests::{base_config, test_selection, test_supervisor};
use super::*;
use std::time::Duration;
use tempfile::TempDir;

/// A config whose create waits `create_delay` first, and whose cleanup
/// writes the environment id it was given to `<dir>/cleaned`.
fn write_config(dir: &Path, create_delay: &str) -> std::path::PathBuf {
    let path = dir.join("config.json");
    let create = format!(
        r#"sleep {create_delay}; printf '{{"environment_id":"env-1","workspace_path":"/tmp/ws","socket_path":"/tmp/s.sock","metadata":{{}}}}'"#
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

async fn launch(
    dir: &Path,
    registry: &EnvironmentRegistry,
    create_delay: &str,
) -> Result<PreparedChild, DomainError> {
    let mut config = base_config(ContainerSelection::New {
        container_config: Some("box".into()),
        name: None,
    });
    config.config_path = Some(write_config(dir, create_delay));
    spawn_prepared_child(
        &config,
        &ChildCommand {
            swarm_context: None,
            supervisor: &test_supervisor(),
            binary: Path::new("true"),
            cli_args: &[],
            base_dir: dir,
            admission_dir: None,
        },
        registry,
        Some(&test_selection(dir)),
    )
    .await
}

fn cleaned(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("cleaned")).ok()
}

async fn rollbacks_settle() {
    assert!(super::super::launch_rollbacks::settled(Duration::from_secs(30)).await);
}

#[tokio::test]
async fn a_spawn_dropped_during_its_create_removes_the_container_it_made() {
    let dir = TempDir::new().unwrap();
    let registry = EnvironmentRegistry::new();
    let stopped = tokio::time::timeout(
        Duration::from_millis(100),
        launch(dir.path(), &registry, "0.5"),
    )
    .await;
    assert!(stopped.is_err(), "the create outlasts the wait");
    assert_eq!(cleaned(dir.path()), None, "the create has not finished");
    rollbacks_settle().await;
    assert_eq!(cleaned(dir.path()).as_deref(), Some("env-1"));
    assert!(registry.get("C1").is_none(), "the record is gone");
}

#[tokio::test]
async fn a_prepared_launch_dropped_before_registration_rolls_back() {
    let dir = TempDir::new().unwrap();
    let registry = EnvironmentRegistry::new();
    let prepared = launch(dir.path(), &registry, "0").await.unwrap();
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
    let mut prepared = launch(dir.path(), &registry, "0").await.unwrap();
    prepared.hand_over();
    drop(prepared);
    rollbacks_settle().await;
    assert_eq!(cleaned(dir.path()), None);
    assert!(registry.get("C1").is_some());
}
