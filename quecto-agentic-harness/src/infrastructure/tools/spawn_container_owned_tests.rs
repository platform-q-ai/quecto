//! Continued `spawn_container` tests (split for the per-file line gate).
use super::tests::{base_config, parse_create, parse_exec, test_record, test_supervisor};
use super::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[tokio::test]
async fn rollback_kills_child_and_consumes_cleanup_once() {
    let registry = EnvironmentRegistry::new();
    registry.commit(test_record("C-test", "env-test"));
    let supervisor = test_supervisor();
    let mut sleep = tokio::process::Command::new("sleep");
    sleep.arg("30");
    let spawned = supervisor
        .spawn(sleep, ProcessGroup::Inherited)
        .await
        .unwrap();
    let mut prepared = PreparedChild {
        swarm_reservation: None,
        owned_child: Some(spawned.handle),
        display_pid: spawned.display_pid.0,
        supervisor: supervisor.clone(),
        environment_ref: Some("C-test".into()),
        endpoint: None,
        proxy_bridge: None,
        process_owner: crate::infrastructure::tools::process_tree::ProcessOwner::DirectPid,
        cleanup_environment_id: Some("env-test".into()),
        cleanup_argv: vec!["true".into()],
        environments: Some(registry.clone()),
    };
    prepared.rollback_once().await;
    assert!(prepared.cleanup_argv.is_empty());
    assert!(registry.get("C-test").is_none());
}

#[tokio::test]
async fn script_managed_child_success_sets_environment_ref_and_cleanup() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"{
  "container_configs": {
    "default": {"default": true, "create": ["printf", "{\"environment_id\":\"env-1\",\"workspace_path\":\"/tmp/ws\",\"metadata\":{},\"socket_path\":\"/tmp/child.sock\"}"], "cleanup": ["true"]}
  }
}
"#,
    )
    .unwrap();
    let mut config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    config.config_path = Some(cfg_path);
    let registry = EnvironmentRegistry::new();
    let prepared = spawn_prepared_child(
        &config,
        &ChildCommand {
            swarm_context: None,
            supervisor: &test_supervisor(),
            binary: Path::new("true"),
            cli_args: &[],
            base_dir: dir.path(),
            admission_dir: None,
        },
        &registry,
        None,
    )
    .await
    .unwrap();
    assert_eq!(prepared.environment_ref.as_deref(), Some("C1"));
    let committed = registry.get("C1").unwrap();
    assert_eq!(committed.environment_id, "env-1");
    assert_eq!(committed.workspace_path, PathBuf::from("/tmp/ws"));
    assert_eq!(committed.script_name, "default");
    // No repository in the script's metadata -> sandbox listing (#1410).
    assert_eq!(committed.repository, "");
    let (env_ref, argv) = prepared.cleanup_plan();
    assert!(env_ref.as_deref().unwrap() == "env-1");
    assert_eq!(argv, vec!["true"]);
    assert!(prepared.owned_child.is_none());
}

#[test]
fn create_result_contract_rejects_invalid_shapes_and_proxy() {
    assert!(parse_create(b"").is_err());
    let unknown_key = br#"{"environment_id":"e-unknown","workspace_path":"/tmp/ws","metadata":{},"socket_path":"/tmp/sock","bogus":1}"#;
    assert!(parse_create(unknown_key).is_err());
    assert_eq!(
        salvage_environment_id(unknown_key).as_deref(),
        Some("e-unknown")
    );
    assert!(salvage_environment_id(br#"{"no_id":true}"#).is_none());
    assert!(salvage_environment_id(br#"{"environment_id":""}"#).is_none());
    let trailing =
        br#"{"environment_id":"e-trail","workspace_path":"/tmp/ws","metadata":{},"socket_path":"/tmp/sock"} extra"#;
    assert!(parse_create(trailing).is_err());
    assert_eq!(salvage_environment_id(trailing).as_deref(), Some("e-trail"));
    assert!(
        parse_create(
            br#"{"environment_id":"e","workspace_path":"/tmp/ws","metadata":{},"socket_proxy":{}}"#
        )
        .is_err()
    );
    assert!(parse_create(br#"{"environment_id":"e","workspace_path":"/tmp/ws","metadata":[],"socket_path":"/tmp/sock"}"#).is_err());
}

#[test]
fn create_result_contract_accepts_direct_endpoint() {
    let parsed = parse_create(
        br#"{"environment_id":"env","workspace_path":"/tmp/ws","metadata":{"k":"v"},"socket_path":"/tmp/sock"}"#,
    )
    .unwrap();
    assert_eq!(parsed.environment_id, "env");
    assert_eq!(
        parsed.endpoint,
        crate::domain::subagent_launch::ParentEndpoint::Direct {
            socket_path: PathBuf::from("/tmp/sock")
        }
    );
}

#[test]
fn unsafe_arg_detects_empty_and_nul_only() {
    assert!(unsafe_arg(""));
    assert!(unsafe_arg("bad\0arg"));
    assert!(!unsafe_arg("safe-arg"));
}

#[test]
fn exec_result_contract_rejects_invalid_shapes_and_proxy() {
    assert!(parse_exec(b"").is_err());
    assert!(parse_exec(br#"{"metadata":{},"socket_path":""}"#).is_err());
    assert!(parse_exec(br#"{"metadata":{},"socket_proxy":{},"socket_path":"/tmp/s"}"#).is_err());
    assert!(parse_exec(br#"{"metadata":[],"socket_path":"/tmp/s"}"#).is_err());
    assert!(parse_exec(br#"{"metadata":{},"socket_path":"/tmp/s"} extra"#).is_err());
    assert!(parse_exec(br#"{"metadata":{},"socket_path":"/tmp/s","bogus":1}"#).is_err());
    assert_eq!(
        parse_exec(br#"{"metadata":{},"socket_path":"/tmp/s"}"#).unwrap(),
        crate::domain::subagent_launch::ParentEndpoint::Direct {
            socket_path: PathBuf::from("/tmp/s")
        }
    );
}

#[test]
fn environment_name_is_taken_from_new_mode_only() {
    let named = base_config(ContainerSelection::New {
        container_config: None,
        name: Some("review-env".into()),
    });
    assert_eq!(environment_name(&named).as_deref(), Some("review-env"));
    assert!(environment_name(&base_config(ContainerSelection::Local)).is_none());
}

#[tokio::test]
async fn join_fails_for_unknown_target_and_missing_retained_exec() {
    let registry = EnvironmentRegistry::new();
    let child = ChildCommand {
        swarm_context: None,
        supervisor: &test_supervisor(),
        binary: Path::new("true"),
        cli_args: &[],
        base_dir: Path::new("/tmp"),
        admission_dir: None,
    };
    // Unknown ref: no exec is attempted.
    let err = join_script_managed_child(
        &child,
        &registry,
        &crate::domain::environment_registry::EnvironmentTarget::Ref("C9".into()),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("unknown"), "{err}");

    // Committed environment without retained exec argv cannot be joined.
    registry.commit(test_record("C1", "env-noexec"));
    let err = join_script_managed_child(
        &child,
        &registry,
        &crate::domain::environment_registry::EnvironmentTarget::Ref("C1".into()),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("retained exec"), "{err}");
}

#[test]
fn explicit_selection_also_fails_at_load_when_no_default_is_labeled() {
    // #1410 accepted trade-off: exactly-one-default is a LOAD invariant, so a
    // config file with entries but no `"default": true` label blocks ALL
    // container spawns — including explicitly named selection, which needs no
    // default. Explicit selection is only reachable through a valid config.
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(
        &cfg_path,
        r#"{"container_configs":{"a":{"create":["c"],"cleanup":["k"]}}}"#,
    )
    .unwrap();
    let mut config = base_config(ContainerSelection::New {
        container_config: Some("a".into()),
        name: None,
    });
    config.config_path = Some(cfg_path);
    let err = load_container_config(&config, None, Path::new("/tmp"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("no container config is labeled"), "{err}");
    assert!(err.contains("a"), "error must enumerate names: {err}");
}

/// A rolled-back launch (no reaper ever registered) leaves no slot behind.
#[tokio::test]
async fn a_failed_launch_leaves_no_supervisor_slot() {
    let mut sleep = tokio::process::Command::new("sleep");
    sleep.arg("30");
    let mut prepared = PreparedChild::new_for_test(Some(sleep), None, None).await;
    let supervisor = prepared.supervisor.clone();
    assert_eq!(supervisor.slot_count(), 1);
    prepared.rollback_once().await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while supervisor.slot_count() > 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(supervisor.slot_count(), 0);
}
