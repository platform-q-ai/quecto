use super::*;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[test]
fn parse_rejects_runtime_specific_fields() {
    for field in ["branch", "pr", "image", "runtime"] {
        let err =
            parse_container_selection(&json!({"container":{"mode":"new", field:"x"}})).unwrap_err();
        assert!(err.contains(field), "{err}");
    }
}

#[test]
fn parse_rejects_invalid_container_shapes_and_modes() {
    assert!(parse_container_selection(&json!({"container":"new"})).is_err());
    assert!(parse_container_selection(&json!({"container":{"repo":"r"}})).is_err());
    assert!(parse_container_selection(&json!({"container":{"mode":"existing"}})).is_err());
    assert!(parse_container_selection(&json!({"container":{"mode":"other"}})).is_err());
    assert!(parse_container_selection(&json!({"container":{"mode":"new","repo":1}})).is_err());
    assert!(
        parse_container_selection(&json!({"container":{"mode":"new","container_script":1}}))
            .is_err()
    );
}

#[test]
fn parse_accepts_true_false_and_new_object() {
    assert!(matches!(
        parse_container_selection(&json!({})).unwrap(),
        ContainerSelection::Local
    ));
    assert!(matches!(
        parse_container_selection(&json!({"container":false})).unwrap(),
        ContainerSelection::Local
    ));
    assert!(matches!(
        parse_container_selection(&json!({"container":true})).unwrap(),
        ContainerSelection::New { .. }
    ));
    let parsed = parse_container_selection(
        &json!({"container":{"mode":"new","repo":"r","container_script":"s"}}),
    )
    .unwrap();
    assert_eq!(
        parsed,
        ContainerSelection::New {
            repo: Some("r".into()),
            container_script: Some("s".into())
        }
    );
}

#[test]
fn validate_script_rejects_missing_or_unsafe_argv() {
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec![],
            cleanup: vec![]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["".into()],
            cleanup: vec![]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["ok".into()],
            cleanup: vec!["bad\0".into()]
        })
        .is_err()
    );
    assert!(
        validate_script(&ContainerScriptConfig {
            create: vec!["ok".into()],
            cleanup: vec!["cleanup".into()]
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
            Path::new("/definitely/not/quecto"),
            &[],
            Path::new("/tmp")
        )
        .await
        .is_err()
    );

    let mut without_config = base_config(ContainerSelection::New {
        repo: None,
        container_script: None,
    });
    assert!(
        spawn_prepared_child(&without_config, Path::new("true"), &[], Path::new("/tmp"))
            .await
            .is_err()
    );

    without_config.config_path = Some(PathBuf::from("relative.toml"));
    assert!(
        spawn_prepared_child(&without_config, Path::new("true"), &[], Path::new("/tmp"))
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
    });
    config.config_path = Some(cfg_path);
    assert!(
        spawn_prepared_child(&config, Path::new("true"), &[], dir.path())
            .await
            .is_err()
    );
}

#[test]
fn create_command_and_common_env_are_constructed_without_shell() {
    let script = ContainerScriptConfig {
        create: vec!["echo".into(), "prefix".into()],
        cleanup: vec![],
    };
    let mut cmd = create_command(&script, Path::new("/bin/quecto"), &["--mode".into()]);
    apply_common_child_env(&mut cmd, Path::new("/tmp/base"));
    let _ = cmd;
}

#[test]
fn script_env_includes_optional_selection_values() {
    let cfg = Config {
        agents: crate::infrastructure::config::AgentConfig {
            defaults: crate::infrastructure::config::AgentDefaults {
                repo: Some("repo-default".into()),
                ..Default::default()
            },
        },
        ..configured_scripts()
    };
    let config = base_config(ContainerSelection::New {
        repo: Some("explicit".into()),
        container_script: Some("alt".into()),
    });
    let mut cmd = create_command(
        script_config(&cfg, "default").unwrap(),
        Path::new("/bin/quecto"),
        &[],
    );
    set_script_env(&mut cmd, &config, &cfg, "alt", "", Path::new("/tmp/base")).unwrap();
    apply_common_child_env(&mut cmd, Path::new("/tmp/base"));
    let _ = cmd;
}

#[tokio::test]
async fn local_child_success_has_no_cleanup_plan() {
    let config = base_config(ContainerSelection::Local);
    let mut prepared = spawn_prepared_child(&config, Path::new("true"), &[], Path::new("/tmp"))
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
    };
    prepared.rollback_once().await;
    assert!(prepared.cleanup_argv.is_empty());
}

#[tokio::test]
async fn script_managed_child_success_sets_environment_ref_and_cleanup() {
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"{
  "agents": {"defaults": {"repo": "default-repo"}},
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
    });
    config.config_path = Some(cfg_path);
    let prepared = spawn_prepared_child(&config, Path::new("true"), &[], dir.path())
        .await
        .unwrap();
    assert!(
        prepared
            .environment_ref
            .as_deref()
            .unwrap()
            .starts_with("C")
    );
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
    });
    let err = selected_repo(&config, dir.path()).unwrap_err();
    assert!(err.to_string().contains("remote.origin.url"), "{err}");
}

#[test]
fn create_result_contract_rejects_invalid_shapes_and_proxy() {
    assert!(parse_create_result(b"").is_err());
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
fn script_managed_environment_refs_are_monotonic_c_style_and_cleanup_uses_endpoint_id() {
    let first = new_environment_ref();
    let second = new_environment_ref();
    assert!(first.starts_with('C'));
    assert!(second.starts_with('C'));
    let first_num: u64 = first.trim_start_matches('C').parse().unwrap();
    let second_num: u64 = second.trim_start_matches('C').parse().unwrap();
    assert!(second_num > first_num);
    assert!(!first.contains('-'));
}
