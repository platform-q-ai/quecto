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
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A composed container-config selection with no launching-agent source:
/// only an explicit spawn `config` file supplies entries (#2024 S4a).
pub(super) fn test_selection(
    base_dir: &Path,
) -> Arc<crate::application::subagents::use_cases::SelectContainerConfig> {
    crate::composition::container_configs::build_container_config_selection(base_dir, None)
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
        origin: crate::domain::environment_registry::EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
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

#[tokio::test]
async fn a_new_container_needs_a_composed_selection_but_a_local_child_does_not() {
    let dir = TempDir::new().unwrap();
    let config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    let err = spawn_prepared_child(
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
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("tool error: {NO_CONTAINER_CONFIG_SELECTION_COMPOSED}")
    );
    let local = base_config(ContainerSelection::Local);
    assert!(
        spawn_prepared_child(
            &local,
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
        .is_ok()
    );
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

/// The selection's refusals reach the launch as tool errors, in the use
/// case's words, before any script runs.
#[tokio::test]
async fn selection_errors_surface_as_tool_errors_without_a_launch() {
    let dir = TempDir::new().unwrap();
    let selection = test_selection(dir.path());
    let mut without_config = base_config(ContainerSelection::New {
        container_config: None,
        name: None,
    });
    let supervisor = test_supervisor();
    let child = || ChildCommand {
        swarm_context: None,
        supervisor: &supervisor,
        binary: Path::new("true"),
        cli_args: &[],
        base_dir: dir.path(),
        admission_dir: None,
    };
    let err = spawn_prepared_child(
        &without_config,
        &child(),
        &EnvironmentRegistry::new(),
        Some(&selection),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("container spawn requires --config so container_configs can be loaded"),
        "{err}"
    );
    without_config.config_path = Some(PathBuf::from("relative.toml"));
    let err = spawn_prepared_child(
        &without_config,
        &child(),
        &EnvironmentRegistry::new(),
        Some(&selection),
    )
    .await
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
        container_diagnostics: Vec::new(),
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
    let selection = test_selection(dir.path());
    let err = spawn_prepared_child(
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
        Some(&selection),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("failed to invoke script-managed create"),
        "{err}"
    );
}

#[test]
fn create_command_and_common_env_are_constructed_without_shell() {
    let create = vec!["echo".to_string(), "prefix".to_string()];
    let mut cmd = script_command(&create, Path::new("/bin/quecto"), &["--mode".into()]);
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
    let selection = test_selection(dir.path());
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
        Some(&selection),
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

/// A named launch from a checkout whose overlay is untrusted carries the
/// overlay's diagnostic on the prepared child (#2024 S4a review M): the
/// launch adapter puts it in the tool result, so the model sees it.
#[tokio::test]
async fn a_named_launch_over_a_withheld_overlay_carries_the_diagnostic() {
    let dir = TempDir::new().unwrap();
    let base = dir.path().join("base");
    let checkout = dir.path().join("checkout");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let create = r#"printf '{"environment_id":"env-1","workspace_path":"/tmp/ws","socket_path":"/tmp/s.sock","metadata":{}}'"#;
    std::fs::write(
        base.join("config.json"),
        serde_json::json!({"container_configs": {
            "global": {"default": true, "create": ["/bin/sh", "-c", create], "cleanup": ["true"]},
            "alt": {"create": ["/bin/sh", "-c", create], "cleanup": ["true"]}
        }})
        .to_string(),
    )
    .unwrap();
    let overlay = checkout.join(".quecto").join("config.json");
    std::fs::write(
        &overlay,
        serde_json::json!({"container_configs": {"repo": {"default": true, "create": ["/bin/true"], "cleanup": ["true"]}}})
            .to_string(),
    )
    .unwrap();
    let selection = crate::composition::container_configs::build_container_config_selection(
        &base,
        Some(
            crate::application::configuration::dto::ConfigSelection::Layered(
                crate::application::configuration::dto::ConfigLayers {
                    global: base.join("config.json"),
                    overlay: Some(overlay.clone()),
                    legacy_local: None,
                },
            ),
        ),
    );
    let child = ChildCommand {
        swarm_context: None,
        supervisor: &test_supervisor(),
        binary: Path::new("true"),
        cli_args: &[],
        base_dir: &base,
        admission_dir: None,
    };
    let registry = EnvironmentRegistry::new();
    // The implicit default is refused: the overlay's default is unknown.
    let err = spawn_prepared_child(
        &base_config(ContainerSelection::New {
            container_config: None,
            name: None,
        }),
        &child,
        &registry,
        Some(&selection),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("container: true refused")
            && err.to_string().contains(&overlay.display().to_string()),
        "{err}"
    );
    assert!(registry.get("C1").is_none(), "no create ran");
    // A name launches from the global set, the diagnostic travelling along.
    let prepared = spawn_prepared_child(
        &base_config(ContainerSelection::New {
            container_config: Some("alt".into()),
            name: None,
        }),
        &child,
        &registry,
        Some(&selection),
    )
    .await
    .unwrap();
    assert_eq!(
        prepared.container_diagnostics.len(),
        1,
        "{:?}",
        prepared.container_diagnostics
    );
    assert!(
        prepared.container_diagnostics[0].contains(&overlay.display().to_string())
            && prepared.container_diagnostics[0].contains("quecto config trust"),
        "{:?}",
        prepared.container_diagnostics
    );
    assert_eq!(registry.get("C1").unwrap().script_name, "alt");
}
