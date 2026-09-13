use super::*;

pub(super) fn test_supervisor() -> Arc<OwnedChildSupervisor> {
    Arc::new(OwnedChildSupervisor::new())
}

pub(super) fn parse_create(stdout: &[u8]) -> Result<CreateResult, DomainError> {
    parse_create_result(stdout, None)
}
pub(super) fn parse_exec(stdout: &[u8]) -> Result<ParentEndpoint, DomainError> {
    parse_exec_result(stdout, None)
}
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[test]
fn validate_script_rejects_missing_or_unsafe_argv() {
    assert!(
        validate_container_config(&ContainerConfig {
            default: false,
            create: vec![],
            cleanup: vec![],
            exec: vec![],
            kill: vec![],
            inspect: vec![],
        })
        .is_err()
    );
    assert!(
        validate_container_config(&ContainerConfig {
            default: false,
            create: vec!["".into()],
            cleanup: vec![],
            exec: vec![],
            kill: vec![],
            inspect: vec![],
        })
        .is_err()
    );
    assert!(
        validate_container_config(&ContainerConfig {
            default: false,
            create: vec!["ok".into()],
            cleanup: vec!["bad\0".into()],
            exec: vec![],
            kill: vec![],
            inspect: vec![],
        })
        .is_err()
    );
    assert!(
        validate_container_config(&ContainerConfig {
            default: false,
            create: vec!["ok".into()],
            cleanup: vec![],
            exec: vec![],
            kill: vec![],
            inspect: vec![],
        })
        .is_err()
    );
    assert!(
        validate_container_config(&ContainerConfig {
            default: false,
            create: vec!["ok".into()],
            cleanup: vec!["cleanup".into()],
            exec: vec![],
            kill: vec![],
            inspect: vec![],
        })
        .is_ok()
    );
}

fn configured_container_configs() -> Config {
    let mut container_configs = HashMap::new();
    container_configs.insert(
        "default".to_string(),
        ContainerConfig {
            default: true,
            create: vec!["echo".into()],
            cleanup: vec!["echo".into()],
            exec: vec![],
            kill: vec![],
            inspect: vec![],
        },
    );
    Config {
        container_configs,
        ..Default::default()
    }
}

pub(super) fn test_record(env_ref: &str, env_id: &str) -> EnvironmentRecord {
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
        retained_inspect_argv: vec![],
        members: vec![],
        status: crate::domain::environment_registry::EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    }
}

pub(super) fn base_config(container: ContainerSelection) -> SubagentConfig {
    SubagentConfig {
        container,
        task: None,
        agent_id: None,
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
fn container_config_selection_uses_the_default_label_and_enumerates_on_errors() {
    let cfg = configured_container_configs();
    // Omitted selection resolves to the entry labeled `"default": true`.
    assert_eq!(container_config_name(&None, &cfg).unwrap(), "default");
    assert!(container_config(&cfg, "default").is_ok());

    // Unknown names enumerate the configured menu so agents can offer it.
    let err = container_config(&cfg, "missing").unwrap_err().to_string();
    assert!(err.contains("unknown container config 'missing'"), "{err}");
    assert!(
        err.contains("available container configs: default"),
        "{err}"
    );

    // Defensive-arm only: in production Config::load rejects a non-empty map
    // with no default before this code runs; the arm still enumerates so a
    // future bypass cannot fail silently.
    let mut unlabeled = configured_container_configs();
    unlabeled
        .container_configs
        .get_mut("default")
        .unwrap()
        .default = false;
    let err = container_config_name(&None, &unlabeled)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no container config is labeled"), "{err}");
    assert!(
        err.contains("available container configs: default"),
        "{err}"
    );

    // An explicitly selected name wins regardless of labels.
    assert_eq!(
        container_config_name(&Some("other".into()), &cfg).unwrap(),
        "other"
    );
}

#[test]
fn local_selection_has_no_container_config_requirement() {
    let config = base_config(ContainerSelection::Local);
    assert!(load_container_config(&config, None, Path::new("/tmp")).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn local_subagent_inherits_parent_process_group() {
    let dir = TempDir::new().unwrap();
    let cli_args = vec![std::ffi::OsString::from("2")];
    let mut prepared = spawn_local_child(&ChildCommand {
        swarm_context: None,
        supervisor: &test_supervisor(),
        binary: Path::new("/bin/sleep"),
        cli_args: &cli_args,
        base_dir: dir.path(),
        admission_dir: None,
    })
    .await
    .expect("local child should spawn");

    assert!(prepared.owned_child.is_some(), "local launch owns child");
    let pid = prepared.display_pid as libc::pid_t;
    // SAFETY: `pid` comes from a live child process we just spawned.
    let child_pgid = unsafe { libc::getpgid(pid) };
    // SAFETY: `getpgrp` reads the current process group and has no preconditions.
    let parent_pgid = unsafe { libc::getpgrp() };

    prepared.rollback_once().await;

    assert_eq!(
        child_pgid, parent_pgid,
        "local subagents must stay in the parent agent's PGID so TUI ordinary-exit cleanup kills the whole spawned tree"
    );
    assert_ne!(
        child_pgid, pid,
        "local subagents must not create their own process group"
    );
}

#[test]
fn relative_config_path_is_rejected_for_container_config() {
    let mut config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    config.config_path = Some(PathBuf::from("relative.toml"));
    assert!(load_container_config(&config, None, Path::new("/tmp")).is_err());
}

fn write_container_configs(dir: &std::path::Path, default: &str) -> PathBuf {
    let path = dir.join(format!("config-{default}.json"));
    let config = serde_json::json!({
        "container_configs": {
            default: {"default": true, "create": ["/bin/true"], "cleanup": ["/bin/true"]}
        }
    });
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    path
}

#[test]
fn parent_config_path_is_the_fallback_when_spawn_config_is_omitted() {
    let dir = TempDir::new().unwrap();
    let parent = write_container_configs(dir.path(), "parentset");
    let config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    let loaded = load_container_config(&config, Some(&parent), Path::new("/tmp")).unwrap();
    assert_eq!(container_config_name(&None, &loaded).unwrap(), "parentset");
}

#[test]
fn explicit_spawn_config_wins_over_the_parent_config_path() {
    let dir = TempDir::new().unwrap();
    let parent = write_container_configs(dir.path(), "parentset");
    let explicit = write_container_configs(dir.path(), "explicitset");
    let mut config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    config.config_path = Some(explicit);
    let loaded = load_container_config(&config, Some(&parent), Path::new("/tmp")).unwrap();
    assert_eq!(
        container_config_name(&None, &loaded).unwrap(),
        "explicitset"
    );
}

#[test]
fn roster_config_loader_uses_read_only_trust_and_ignores_unapproved_repo_local_config() {
    let dir = TempDir::new().unwrap();
    let parent = write_container_configs(dir.path(), "parentset");
    let checkout = dir.path().join("checkout");
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    std::fs::write(
        checkout.join(".quecto/config.json"),
        r#"{"container_configs":{"localset":{"default":true,"create":["/bin/false"],"cleanup":["/bin/true"]}}}"#,
    )
    .unwrap();
    let config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });

    let loaded = load_container_config_for_roster(&config, Some(&parent), &checkout).unwrap();

    assert_eq!(container_config_name(&None, &loaded).unwrap(), "parentset");
    assert!(!loaded.container_configs.contains_key("localset"));
}

#[test]
fn relative_parent_config_path_is_rejected_for_container_config() {
    let config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    let err = load_container_config(&config, Some(Path::new("relative.toml")), Path::new("/tmp"))
        .unwrap_err();
    assert!(err.to_string().contains("absolute"), "{err}");
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
    let supervisor = test_supervisor();
    let spawned = supervisor
        .spawn(
            tokio::process::Command::new("true"),
            ProcessGroup::Inherited,
        )
        .await
        .unwrap();
    let prepared = PreparedChild {
        swarm_reservation: None,
        owned_child: Some(spawned.handle),
        display_pid: spawned.display_pid.0,
        supervisor: supervisor.clone(),
        environment_ref: Some("C-test".into()),
        endpoint: None,
        proxy_bridge: None,
        process_owner: crate::infrastructure::tools::process_tree::ProcessOwner::DirectPid,
        cleanup_environment_id: Some("env-test".into()),
        cleanup_argv: vec!["echo".into(), "ok".into()],
        environments: None,
        stderr_tail: None,
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
                swarm_context: None,
                supervisor: &test_supervisor(),
                binary: Path::new("/definitely/not/quecto"),
                cli_args: &[],
                base_dir: Path::new("/tmp"),
                admission_dir: None,
            },
            &EnvironmentRegistry::new(),
            None,
        )
        .await
        .is_err()
    );

    let mut without_config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    assert!(
        spawn_prepared_child(
            &without_config,
            &ChildCommand {
                swarm_context: None,
                supervisor: &test_supervisor(),
                binary: Path::new("true"),
                cli_args: &[],
                base_dir: Path::new("/tmp"),
                admission_dir: None,
            },
            &EnvironmentRegistry::new(),
            None,
        )
        .await
        .is_err()
    );

    without_config.config_path = Some(PathBuf::from("relative.toml"));
    assert!(
        spawn_prepared_child(
            &without_config,
            &ChildCommand {
                swarm_context: None,
                supervisor: &test_supervisor(),
                binary: Path::new("true"),
                cli_args: &[],
                base_dir: Path::new("/tmp"),
                admission_dir: None,
            },
            &EnvironmentRegistry::new(),
            None,
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
        r#"{
  "container_configs": {
    "default": {"default": true, "create": ["/definitely/not/script"], "cleanup": ["echo"]}
  }
}
"#,
    )
    .unwrap();
    let mut config = base_config(ContainerSelection::New {
        container_config: Some("default".into()),
        name: None,
    });
    config.config_path = Some(cfg_path);
    assert!(
        spawn_prepared_child(
            &config,
            &ChildCommand {
                swarm_context: None,
                supervisor: &test_supervisor(),
                binary: Path::new("true"),
                cli_args: &[],
                base_dir: dir.path(),
                admission_dir: None,
            },
            &EnvironmentRegistry::new(),
            None,
        )
        .await
        .is_err()
    );
}

#[test]
fn create_command_and_common_env_are_constructed_without_shell() {
    let script = ContainerConfig {
        default: false,
        create: vec!["echo".into(), "prefix".into()],
        cleanup: vec![],
        exec: vec![],
        kill: vec![],
        inspect: vec![],
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
  "container_configs": {
    "alt": {"default": true, "create": ["/bin/sh", "-c", "printf '{\"environment_id\":\"env-env\",\"workspace_path\":\"/tmp/ws\",\"socket_path\":\"/tmp/s.sock\",\"metadata\":{\"config\":\"'\"$QUECTO_CONTAINER_CONFIG\"'\",\"ref\":\"'\"$QUECTO_CONTAINER_ENVIRONMENT_REF\"'\",\"repository\":\"https://example.invalid/baked.git\"}}'"], "cleanup": ["true"]}
  }
}
"#,
    )
    .unwrap();
    let mut config = base_config(ContainerSelection::New {
        container_config: Some("alt".into()),
        name: None,
    });
    config.config_path = Some(cfg_path);
    let registry = EnvironmentRegistry::new();
    spawn_prepared_child(
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
    let committed = registry.get("C1").unwrap();
    assert_eq!(
        committed.metadata,
        serde_json::json!({
            "config": "alt",
            "ref": "C1",
            "repository": "https://example.invalid/baked.git",
        })
    );
    // The listing/TUI repository comes from the script's own report (#1410).
    assert_eq!(committed.repository, "https://example.invalid/baked.git");
    assert_eq!(committed.retained_cleanup_argv, vec!["true"]);
}

#[tokio::test]
async fn local_child_success_has_no_cleanup_plan() {
    let config = base_config(ContainerSelection::Local);
    let prepared = spawn_prepared_child(
        &config,
        &ChildCommand {
            swarm_context: None,
            supervisor: &test_supervisor(),
            binary: Path::new("true"),
            cli_args: &[],
            base_dir: Path::new("/tmp"),
            admission_dir: None,
        },
        &EnvironmentRegistry::new(),
        None,
    )
    .await
    .unwrap();
    let (env_ref, argv) = prepared.cleanup_plan();
    assert!(env_ref.is_none());
    assert!(argv.is_empty());
    assert!(prepared.wait_owned_child_exit().await.is_some());
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
