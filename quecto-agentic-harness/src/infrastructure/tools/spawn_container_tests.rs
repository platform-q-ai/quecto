use super::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[test]
fn validate_script_rejects_missing_or_unsafe_argv() {
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec![],
            cleanup: vec![],
            exec: vec![],
            kill: vec![]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["".into()],
            cleanup: vec![],
            exec: vec![],
            kill: vec![]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["ok".into()],
            cleanup: vec!["bad\0".into()],
            exec: vec![],
            kill: vec![]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["ok".into()],
            cleanup: vec![],
            exec: vec![],
            kill: vec![]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["ok".into()],
            cleanup: vec!["cleanup".into()],
            exec: vec![],
            kill: vec![]
        })
        .is_ok()
    );
}

fn configured_scripts() -> Config {
    let mut scripts = HashMap::new();
    scripts.insert(
        "default".to_string(),
        ContainerScriptConfig {
            create: vec!["echo".into()],
            cleanup: vec!["echo".into()],
            exec: vec![],
            kill: vec![],
        },
    );
    Config {
        container_scripts: crate::infrastructure::config::ContainerScriptsConfig {
            default: "default".into(),
            scripts,
        },
        ..Default::default()
    }
}

fn test_record(env_ref: &str, env_id: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.into(),
        environment_id: env_id.into(),
        environment_uuid: crate::domain::environment_registry::mint_environment_uuid(),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        members: vec![],
        status: crate::domain::environment_registry::EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    }
}

fn base_config(container: ContainerSelection) -> SubagentConfig {
    SubagentConfig {
        container,
        task: None,
        agent_id: None,
        restrict_to_workspace: true,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    }
}

#[test]
fn script_selection_and_repo_defaults_are_resolved() {
    let cfg = configured_scripts();
    assert_eq!(script_name(&None, &cfg).unwrap(), "default");
    assert!(script_config(&cfg, "default").is_ok());
    assert!(script_config(&cfg, "missing").is_err());
    assert!(script_name(&Some("".into()), &cfg).is_err());

    let config = base_config(ContainerSelection::New {
        repo: Some("explicit".into()),
        container_script: None,
        name: None,
    });
    assert_eq!(
        selected_repo(&config, Path::new("/tmp"))
            .unwrap()
            .as_deref(),
        Some("explicit")
    );
}

#[test]
fn local_selection_has_no_repo_or_container_config_requirement() {
    let config = base_config(ContainerSelection::Local);
    assert!(selected_repo(&config, Path::new("/tmp")).unwrap().is_none());
    assert!(load_container_config(&config).is_err());
}

#[test]
fn relative_config_path_is_rejected_for_container_config() {
    let mut config = base_config(ContainerSelection::New {
        repo: None,
        container_script: None,
        name: None,
    });
    config.config_path = Some(PathBuf::from("relative.toml"));
    assert!(load_container_config(&config).is_err());
}

#[test]
fn cleanup_command_is_once_consumable_by_prepared_child() {
    let cmd = cleanup_command(Some("C-test"), &["echo".into()]);
    assert!(cmd.is_some());
    let cmd = cleanup_command(Some("C-test"), &["echo".into(), "ok".into()]);
    assert!(cmd.is_some());
    assert!(cleanup_command(None, &["echo".into()]).is_none());
    assert!(cleanup_command(Some("C-test"), &[]).is_none());
}

#[tokio::test]
async fn run_cleanup_once_ignores_missing_env_or_empty_argv() {
    let mut argv = vec!["true".into()];
    run_cleanup_once(None, &mut argv).await;
    assert_eq!(argv, vec!["true"]);
    run_cleanup_once(Some("env".into()), &mut argv).await;
    assert!(argv.is_empty());
    run_cleanup_once(Some("env".into()), &mut argv).await;
    assert!(argv.is_empty());
}

#[tokio::test]
async fn cleanup_plan_clones_environment_and_argv() {
    let prepared = PreparedChild {
        child: Some(tokio::process::Command::new("true").spawn().unwrap()),
        environment_ref: Some("C-test".into()),
        socket_path: None,
        cleanup_environment_id: Some("env-test".into()),
        cleanup_argv: vec!["echo".into(), "ok".into()],
        environments: None,
    };
    let (env_ref, argv) = prepared.cleanup_plan();
    assert_eq!(env_ref.as_deref(), Some("env-test"));
    assert_eq!(argv, vec!["echo", "ok"]);
}

#[tokio::test]
async fn local_child_and_container_errors_cover_spawn_paths() {
    let local = base_config(ContainerSelection::Local);
    assert!(
        spawn_prepared_child(
            &local,
            &ChildCommand {
                binary: Path::new("/definitely/not/quecto"),
                cli_args: &[],
                base_dir: Path::new("/tmp"),
            },
            &EnvironmentRegistry::new()
        )
        .await
        .is_err()
    );

    let mut without_config = base_config(ContainerSelection::New {
        repo: None,
        container_script: None,
        name: None,
    });
    assert!(
        spawn_prepared_child(
            &without_config,
            &ChildCommand {
                binary: Path::new("true"),
                cli_args: &[],
                base_dir: Path::new("/tmp"),
            },
            &EnvironmentRegistry::new()
        )
        .await
        .is_err()
    );

    without_config.config_path = Some(PathBuf::from("relative.toml"));
    assert!(
        spawn_prepared_child(
            &without_config,
            &ChildCommand {
                binary: Path::new("true"),
                cli_args: &[],
                base_dir: Path::new("/tmp"),
            },
            &EnvironmentRegistry::new()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn script_managed_spawn_error_uses_config_and_selected_script() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"
[container_scripts]
default = "default"
[container_scripts.scripts.default]
create = ["/definitely/not/script"]
cleanup = ["echo"]
"#,
    )
    .unwrap();
    let mut config = base_config(ContainerSelection::New {
        repo: None,
        container_script: Some("default".into()),
        name: None,
    });
    config.config_path = Some(cfg_path);
    assert!(
        spawn_prepared_child(
            &config,
            &ChildCommand {
                binary: Path::new("true"),
                cli_args: &[],
                base_dir: dir.path(),
            },
            &EnvironmentRegistry::new()
        )
        .await
        .is_err()
    );
}

#[test]
fn create_command_and_common_env_are_constructed_without_shell() {
    let script = ContainerScriptConfig {
        create: vec!["echo".into(), "prefix".into()],
        cleanup: vec![],
        exec: vec![],
        kill: vec![],
    };
    let mut cmd = script_command(&script.create, Path::new("/bin/quecto"), &["--mode".into()]);
    apply_common_child_env(&mut cmd, Path::new("/tmp/base"));
    let std_cmd = cmd.as_std();
    assert_eq!(std_cmd.get_program(), "echo");
    let args: Vec<_> = std_cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // Argv-exec contract: script args, `--` separator, child binary, child args.
    assert_eq!(args, vec!["prefix", "--", "/bin/quecto", "--mode"]);
    let envs: Vec<_> = std_cmd
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect();
    assert!(envs.contains(&("QUECTO_BASE_DIR".to_string(), Some("/tmp/base".to_string()))));
}

/// The production create path (not the test) must wire the selection env
/// vars: deleting the `cmd.env` lines in `spawn_script_managed_child` fails
/// this test (#1390 review finding).
#[tokio::test]
async fn script_env_includes_optional_selection_values() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    // The create script echoes the env vars it received back into metadata.
    std::fs::write(
        &cfg_path,
        r#"{
  "container_scripts": {
    "default": "alt",
    "scripts": {"alt": {"create": ["/bin/sh", "-c", "printf '{\"environment_id\":\"env-env\",\"workspace_path\":\"/tmp/ws\",\"socket_path\":\"/tmp/s.sock\",\"metadata\":{\"repo\":\"'\"$QUECTO_CONTAINER_REPO\"'\",\"script\":\"'\"$QUECTO_CONTAINER_SCRIPT\"'\",\"ref\":\"'\"$QUECTO_CONTAINER_ENVIRONMENT_REF\"'\"}}'"], "cleanup": ["true"]}}
  }
}
"#,
    )
    .unwrap();
    let mut config = base_config(ContainerSelection::New {
        repo: Some("https://example.invalid/explicit.git".into()),
        container_script: Some("alt".into()),
        name: None,
    });
    config.config_path = Some(cfg_path);
    let registry = EnvironmentRegistry::new();
    spawn_prepared_child(
        &config,
        &ChildCommand {
            binary: Path::new("true"),
            cli_args: &[],
            base_dir: dir.path(),
        },
        &registry,
    )
    .await
    .unwrap();
    let committed = registry.get("C1").unwrap();
    assert_eq!(
        committed.metadata,
        serde_json::json!({
            "repo": "https://example.invalid/explicit.git",
            "script": "alt",
            "ref": "C1",
        })
    );
    assert_eq!(committed.retained_cleanup_argv, vec!["true"]);
}

#[tokio::test]
async fn local_child_success_has_no_cleanup_plan() {
    let config = base_config(ContainerSelection::Local);
    let mut prepared = spawn_prepared_child(
        &config,
        &ChildCommand {
            binary: Path::new("true"),
            cli_args: &[],
            base_dir: Path::new("/tmp"),
        },
        &EnvironmentRegistry::new(),
    )
    .await
    .unwrap();
    let (env_ref, argv) = prepared.cleanup_plan();
    assert!(env_ref.is_none());
    assert!(argv.is_empty());
    let _ = prepared.child.as_mut().unwrap().wait().await;
}

#[tokio::test]
async fn cleanup_runner_consumes_argv_only_when_command_exists() {
    let mut no_env = vec!["true".into()];
    run_cleanup_once(None, &mut no_env).await;
    assert_eq!(no_env, vec!["true"]);

    let mut no_cmd = Vec::new();
    run_cleanup_once(Some("C-test".into()), &mut no_cmd).await;
    assert!(no_cmd.is_empty());

    let mut cmd = vec!["true".into()];
    run_cleanup_once(Some("C-test".into()), &mut cmd).await;
    assert!(cmd.is_empty());
}

#[tokio::test]
async fn rollback_kills_child_and_consumes_cleanup_once() {
    let registry = EnvironmentRegistry::new();
    registry.commit(test_record("C-test", "env-test"));
    let mut prepared = PreparedChild {
        child: Some(
            tokio::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .unwrap(),
        ),
        environment_ref: Some("C-test".into()),
        socket_path: None,
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
  "container_scripts": {
    "default": "default",
    "scripts": {"default": {"create": ["printf", "{\"environment_id\":\"env-1\",\"workspace_path\":\"/tmp/ws\",\"metadata\":{},\"socket_path\":\"/tmp/child.sock\"}"], "cleanup": ["true"]}}
  }
}
"#,
    )
    .unwrap();
    let mut config = base_config(ContainerSelection::New {
        repo: Some("explicit".into()),
        container_script: None,
        name: None,
    });
    config.config_path = Some(cfg_path);
    let registry = EnvironmentRegistry::new();
    let prepared = spawn_prepared_child(
        &config,
        &ChildCommand {
            binary: Path::new("true"),
            cli_args: &[],
            base_dir: dir.path(),
        },
        &registry,
    )
    .await
    .unwrap();
    assert_eq!(prepared.environment_ref.as_deref(), Some("C1"));
    let committed = registry.get("C1").unwrap();
    assert_eq!(committed.environment_id, "env-1");
    assert_eq!(committed.workspace_path, PathBuf::from("/tmp/ws"));
    assert_eq!(committed.script_name, "default");
    let (env_ref, argv) = prepared.cleanup_plan();
    assert!(env_ref.as_deref().unwrap() == "env-1");
    assert_eq!(argv, vec!["true"]);
    assert!(prepared.child.is_none());
}

#[test]
fn selected_repo_discovers_parent_checkout_remote_when_repo_omitted() {
    let dir = TempDir::new().unwrap();
    std::process::Command::new("git")
        .arg("init")
        .arg(dir.path())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["remote", "add", "origin", "https://github.com/org/repo.git"])
        .status()
        .unwrap();
    let config = base_config(ContainerSelection::New {
        repo: None,
        container_script: None,
        name: None,
    });
    assert_eq!(
        selected_repo(&config, dir.path()).unwrap().as_deref(),
        Some("https://github.com/org/repo.git")
    );
}

#[test]
fn selected_repo_fails_before_create_when_parent_remote_missing() {
    let dir = TempDir::new().unwrap();
    let config = base_config(ContainerSelection::New {
        repo: None,
        container_script: None,
        name: None,
    });
    let err = selected_repo(&config, dir.path()).unwrap_err();
    assert!(err.to_string().contains("remote.origin.url"), "{err}");
}

#[test]
fn create_result_contract_rejects_invalid_shapes_and_proxy() {
    assert!(parse_create_result(b"").is_err());
    let unknown_key = br#"{"environment_id":"e-unknown","workspace_path":"/tmp/ws","metadata":{},"socket_path":"/tmp/sock","bogus":1}"#;
    assert!(parse_create_result(unknown_key).is_err());
    assert_eq!(
        salvage_environment_id(unknown_key).as_deref(),
        Some("e-unknown")
    );
    assert!(salvage_environment_id(br#"{"no_id":true}"#).is_none());
    assert!(salvage_environment_id(br#"{"environment_id":""}"#).is_none());
    let trailing =
        br#"{"environment_id":"e-trail","workspace_path":"/tmp/ws","metadata":{},"socket_path":"/tmp/sock"} extra"#;
    assert!(parse_create_result(trailing).is_err());
    assert_eq!(salvage_environment_id(trailing).as_deref(), Some("e-trail"));
    assert!(
        parse_create_result(
            br#"{"environment_id":"e","workspace_path":"/tmp/ws","metadata":{},"socket_proxy":{}}"#
        )
        .is_err()
    );
    assert!(parse_create_result(br#"{"environment_id":"e","workspace_path":"/tmp/ws","metadata":[],"socket_path":"/tmp/sock"}"#).is_err());
}

#[test]
fn create_result_contract_accepts_direct_endpoint() {
    let parsed = parse_create_result(
        br#"{"environment_id":"env","workspace_path":"/tmp/ws","metadata":{"k":"v"},"socket_path":"/tmp/sock"}"#,
    )
    .unwrap();
    assert_eq!(parsed.environment_id, "env");
    assert_eq!(parsed.socket_path, PathBuf::from("/tmp/sock"));
}

#[test]
fn unsafe_arg_detects_empty_and_nul_only() {
    assert!(unsafe_arg(""));
    assert!(unsafe_arg("bad\0arg"));
    assert!(!unsafe_arg("safe-arg"));
}

#[test]
fn exec_result_contract_rejects_invalid_shapes_and_proxy() {
    assert!(parse_exec_result(b"").is_err());
    assert!(parse_exec_result(br#"{"metadata":{},"socket_path":""}"#).is_err());
    assert!(
        parse_exec_result(br#"{"metadata":{},"socket_proxy":{},"socket_path":"/tmp/s"}"#).is_err()
    );
    assert!(parse_exec_result(br#"{"metadata":[],"socket_path":"/tmp/s"}"#).is_err());
    assert!(parse_exec_result(br#"{"metadata":{},"socket_path":"/tmp/s"} extra"#).is_err());
    assert!(parse_exec_result(br#"{"metadata":{},"socket_path":"/tmp/s","bogus":1}"#).is_err());
    assert_eq!(
        parse_exec_result(br#"{"metadata":{},"socket_path":"/tmp/s"}"#).unwrap(),
        PathBuf::from("/tmp/s")
    );
}

#[test]
fn environment_name_is_taken_from_new_mode_only() {
    let named = base_config(ContainerSelection::New {
        repo: None,
        container_script: None,
        name: Some("review-env".into()),
    });
    assert_eq!(environment_name(&named).as_deref(), Some("review-env"));
    assert!(environment_name(&base_config(ContainerSelection::Local)).is_none());
}

#[tokio::test]
async fn join_fails_for_unknown_target_and_missing_retained_exec() {
    let registry = EnvironmentRegistry::new();
    let child = ChildCommand {
        binary: Path::new("true"),
        cli_args: &[],
        base_dir: Path::new("/tmp"),
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
