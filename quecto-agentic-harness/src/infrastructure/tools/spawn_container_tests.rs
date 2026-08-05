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
    assert_eq!(selected_repo(&config, &cfg).as_deref(), Some("explicit"));
}

#[test]
fn local_selection_has_no_repo_or_container_config_requirement() {
    let config = base_config(ContainerSelection::Local);
    assert!(selected_repo(&config, &Config::default()).is_none());
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
    assert!(cleanup_command(None, &["echo".into()]).is_none());
    assert!(cleanup_command(Some("C-test"), &[]).is_none());
}

#[tokio::test]
async fn cleanup_plan_clones_environment_and_argv() {
    let prepared = PreparedChild {
        child: tokio::process::Command::new("true").spawn().unwrap(),
        environment_ref: Some("C-test".into()),
        cleanup_argv: vec!["echo".into(), "ok".into()],
    };
    let (env_ref, argv) = prepared.cleanup_plan();
    assert_eq!(env_ref.as_deref(), Some("C-test"));
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
