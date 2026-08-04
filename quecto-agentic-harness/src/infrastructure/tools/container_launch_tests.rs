use super::container_launch::*;
use super::container_registry;
use crate::domain::container_runtime::{
    ContainerScriptSet, ContainerScriptsConfig, ExistingContainerRef, SpawnContainerRequest,
};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent::SubagentConfig;
use crate::infrastructure::tools::container_registry::{ContainerEntry, ContainerStatus};
use std::collections::HashMap;

fn config_with_container(container: SpawnContainerRequest) -> SubagentConfig {
    SubagentConfig {
        task: None,
        agent_id: None,
        restrict_to_workspace: false,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: vec![],
        read_only: false,
        container,
    }
}

fn script_config(create: String, exec: String) -> ContainerScriptsConfig {
    ContainerScriptsConfig {
        default: Some("dev".into()),
        scripts: HashMap::from([(
            "dev".into(),
            ContainerScriptSet {
                create,
                exec,
                inspect: "true".into(),
                kill: "true".into(),
            },
        )]),
    }
}

#[tokio::test]
async fn new_container_spawn_runs_create_script_and_defaults_repo_from_parent() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("create-env.txt");
    let script = format!(
        "printf '%s' \"$QUECTO_REPO_URL\" > {}; printf '%s' '{{\"environment_id\":\"env-1\",\"workspace_path\":\"/workspace/repo\",\"container_name\":\"devbox\"}}'",
        log.display()
    );
    let ctx = ContainerLaunchContext {
        registry: container_registry::new_container_registry(),
        scripts: script_config(script, "echo exec".into()),
        parent_repo: Some("https://example.test/repo.git".into()),
    };
    let config = SubagentConfig {
        container: SpawnContainerRequest::New {
            container_script: None,
            repo: None,
        },
        task: None,
        agent_id: None,
        restrict_to_workspace: false,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: vec![],
        read_only: false,
    };

    let launch = prepare_container_launch(&ctx, &config, &AgentUuid::new("agent-1"))
        .await
        .unwrap()
        .expect("container launch should be prepared");

    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "https://example.test/repo.git"
    );
    assert_eq!(
        launch.entry.repo_url.as_deref(),
        Some("https://example.test/repo.git")
    );
    assert_eq!(launch.entry.workspace_path, "/workspace/repo");
    assert_eq!(launch.entry.container_name.as_deref(), Some("devbox"));
    assert_eq!(launch.entry.exec_command, "echo exec");
}

#[tokio::test]
async fn missing_container_script_fails_before_create_script_runs() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("should-not-exist");
    let script = format!(
        "touch {}; printf '%s' '{{\"environment_id\":\"env-1\"}}'",
        marker.display()
    );
    let ctx = ContainerLaunchContext {
        registry: container_registry::new_container_registry(),
        scripts: script_config(script, "echo exec".into()),
        parent_repo: None,
    };
    let config = SubagentConfig {
        container: SpawnContainerRequest::New {
            repo: Some("https://example.test/repo.git".into()),
            container_script: Some("missing".into()),
        },
        task: None,
        agent_id: None,
        restrict_to_workspace: false,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: vec![],
        read_only: false,
    };

    let err = prepare_container_launch(&ctx, &config, &AgentUuid::new("agent-1"))
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("container script set 'missing' is not configured"),
        "{err}"
    );
    assert!(
        !marker.exists(),
        "create script must not run when validation fails"
    );
}

#[tokio::test]
async fn existing_container_spawn_reuses_registered_environment_and_exec_command() {
    let ctx = ContainerLaunchContext {
        registry: container_registry::new_container_registry(),
        scripts: script_config(
            "printf '%s' '{\"environment_id\":\"env-1\"}'".into(),
            "echo reuse".into(),
        ),
        parent_repo: None,
    };
    container_registry::register_container(
        &ctx.registry,
        ContainerEntry {
            container_uuid: "env-1".into(),
            container_ref: String::new(),
            container_name: Some("devbox".into()),
            environment_id: "env-1".into(),
            repo_url: Some("https://example.test/repo.git".into()),
            workspace_path: "/workspace/repo".into(),
            status: ContainerStatus::Running,
            agents: vec![],
            script_name: "dev".into(),
            exec_command: "echo reuse".into(),
            inspect_command: "true".into(),
            kill_command: "true".into(),
            socket_path: None,
            socket_proxy: None,
            metadata: serde_json::json!({}),
        },
    );
    let config = config_with_container(SpawnContainerRequest::Existing {
        reference: ExistingContainerRef::Ref("C1".into()),
    });

    let launch = prepare_container_launch(&ctx, &config, &AgentUuid::new("agent-2"))
        .await
        .unwrap()
        .expect("existing container should prepare launch");

    assert_eq!(launch.entry.container_uuid, "env-1");
    assert_eq!(launch.entry.container_ref, "C1");
    assert_eq!(launch.entry.workspace_path, "/workspace/repo");
    assert_eq!(launch.entry.exec_command, "echo reuse");
}

#[tokio::test]
async fn container_exec_command_passes_structured_argv_without_joined_env_or_shell() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("capture.py");
    let log = dir.path().join("argv.json");
    std::fs::write(&script, format!("#!/usr/bin/env python3\nimport json, os, sys\nwith open({:?}, 'w') as f:\n    json.dump({{'argv': sys.argv[1:], 'joined': os.environ.get('QUECTO_CHILD_ARGS')}}, f)\n", log)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    let entry = ContainerEntry {
        container_uuid: "env-argv".into(),
        container_ref: "C1".into(),
        container_name: None,
        environment_id: "env-argv".into(),
        repo_url: None,
        workspace_path: dir.path().to_string_lossy().into_owned(),
        status: ContainerStatus::Running,
        agents: vec![],
        script_name: "dev".into(),
        exec_command: script.to_string_lossy().into_owned(),
        inspect_command: "true".into(),
        kill_command: "true".into(),
        socket_path: None,
        socket_proxy: None,
        metadata: serde_json::json!({}),
    };
    let mut cmd = build_container_exec_command(ContainerExecSpec {
        entry: &entry,
        agent_uuid: &AgentUuid::new("agent-argv"),
        parent_id: None,
        requested_socket_path: &dir.path().join("agent.sock"),
        child_binary: std::path::Path::new("/bin/echo"),
        child_args: &["hello".into(), "two words".into(), "$(touch pwn)".into()],
        prepend_child_binary: true,
    });
    assert!(cmd.status().await.unwrap().success());
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(log).unwrap()).unwrap();
    assert_eq!(v["joined"], serde_json::Value::Null);
    assert_eq!(
        v["argv"],
        serde_json::json!(["--", "/bin/echo", "hello", "two words", "$(touch pwn)"])
    );
}

#[test]
fn reference_create_rejects_leading_dash_repo_before_git_clone() {
    let dir = tempfile::tempdir().unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/container-runtime/create.sh");
    let output = std::process::Command::new("bash")
        .arg(script)
        .env("QUECTO_CONTAINER_ROOT", dir.path())
        .env("QUECTO_AGENT_UUID", "malrepo")
        .arg("--repo")
        .arg("--upload-pack=/tmp/pwn")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unsafe repository"), "{stderr}");
    assert!(!dir.path().join("quecto-malrepo/workspace").exists());
}

#[test]
fn reference_kill_rejects_workspace_outside_managed_root() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    std::fs::write(&victim, "keep").unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/container-runtime/kill.sh");
    let output = std::process::Command::new("bash")
        .arg(script)
        .env("QUECTO_CONTAINER_ROOT", root.path())
        .env("QUECTO_ENVIRONMENT_UUID", "env-safe")
        .env("QUECTO_WORKSPACE_PATH", outside.path().join("workspace"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("outside managed root"), "{stderr}");
    assert_eq!(std::fs::read_to_string(victim).unwrap(), "keep");
}

#[test]
fn spawn_reaper_only_kills_container_for_environment_owner() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/infrastructure/tools/spawn.rs"),
    )
    .unwrap();
    assert!(source.contains("if entry.parent_id.is_none()"));
    assert!(source.contains("kill_container_owner(entry)"));
}
