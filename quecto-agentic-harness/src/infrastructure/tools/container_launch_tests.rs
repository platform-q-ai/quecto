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
    let create = dir.path().join("create.py");
    std::fs::write(&create, format!(
        "#!/usr/bin/env python3\nimport json, os\nopen({:?}, 'w').write(os.environ.get('QUECTO_REPO_URL',''))\nprint(json.dumps({{'environment_id':'env-1','workspace_path':'/workspace/repo','container_name':'devbox'}}))\n",
        log
    ))
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&create, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let script = create.to_string_lossy().into_owned();
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
async fn new_container_accepts_typed_unix_socket_proxy() {
    let dir = tempfile::tempdir().unwrap();
    let proxy = dir.path().join("proxy.sock");
    let create = format!(
        "printf '%s' '{{\"environment_id\":\"env-proxy\",\"workspace_path\":\"/workspace/repo\",\"metadata\":{{}},\"socket_proxy\":\"unix:{}\"}}'",
        proxy.display()
    );
    let ctx = ContainerLaunchContext {
        registry: container_registry::new_container_registry(),
        scripts: script_config(create, "echo exec".into()),
        parent_repo: None,
    };
    let config = config_with_container(SpawnContainerRequest::New {
        container_script: None,
        repo: None,
    });

    let launch = prepare_container_launch(&ctx, &config, &AgentUuid::new("agent-proxy"))
        .await
        .unwrap()
        .expect("proxy launch should prepare");

    assert_eq!(
        launch.entry.socket_proxy.as_deref(),
        Some(format!("unix:{}", proxy.display()).as_str())
    );
}

#[tokio::test]
async fn new_container_rejects_raw_or_unsafe_socket_proxy_values() {
    for proxy in [
        "/tmp/raw.sock",
        "tcp://127.0.0.1:1",
        "unix:relative.sock",
        "unix:/tmp/../evil.sock",
    ] {
        let create = format!(
            "printf '%s' '{{\"environment_id\":\"env-proxy\",\"workspace_path\":\"/workspace/repo\",\"metadata\":{{}},\"socket_proxy\":\"{}\"}}'",
            proxy
        );
        let ctx = ContainerLaunchContext {
            registry: container_registry::new_container_registry(),
            scripts: script_config(create, "echo exec".into()),
            parent_repo: None,
        };
        let config = config_with_container(SpawnContainerRequest::New {
            container_script: None,
            repo: None,
        });

        let err = prepare_container_launch(&ctx, &config, &AgentUuid::new("agent-proxy"))
            .await
            .expect_err("unsafe proxy must fail")
            .to_string();
        assert!(err.contains("socket_proxy"), "{proxy}: {err}");
    }
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
            last_error: None,
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
        last_error: None,
    };
    let mut cmd = build_container_exec_command(ContainerExecSpec {
        entry: &entry,
        agent_uuid: &AgentUuid::new("agent-argv"),
        parent_id: None,
        requested_socket_path: &dir.path().join("agent.sock"),
        child_binary: std::path::Path::new("/bin/echo"),
        child_args: &["hello".into(), "two words".into(), "$(touch pwn)".into()],
        prepend_child_binary: true,
    })
    .unwrap();
    assert!(cmd.status().await.unwrap().success());
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(log).unwrap()).unwrap();
    assert_eq!(v["joined"], serde_json::Value::Null);
    assert_eq!(
        v["argv"],
        serde_json::json!(["--", "/bin/echo", "hello", "two words", "$(touch pwn)"])
    );
}

#[tokio::test]
async fn reference_scripts_emit_outputs_accepted_by_strict_production_parsers() {
    let root = tempfile::tempdir().unwrap();
    let script_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/container-runtime");
    let agent = AgentUuid::new("contract-agent");
    let create = run_script_json(
        &script_root.join("create.sh").display().to_string(),
        None,
        None,
        &agent,
    )
    .await
    .expect("create fixture output parses as JSON");
    validate_script_result(&create).expect("create fixture satisfies strict launch parser");
    assert!(create.socket_path.is_some() ^ create.socket_proxy.is_some());

    let inspect = std::process::Command::new("bash")
        .arg(script_root.join("inspect.sh"))
        .env("QUECTO_CONTAINER_ROOT", root.path())
        .env("QUECTO_ENVIRONMENT_UUID", &create.environment_id)
        .env(
            "QUECTO_WORKSPACE_PATH",
            root.path().join("contract-agent/workspace"),
        )
        .output()
        .unwrap();
    assert!(inspect.status.success());
    super::container_script_cleanup::validate_inspect_output(&inspect.stdout)
        .expect("inspect fixture satisfies strict parser");

    let kill = std::process::Command::new("bash")
        .arg(script_root.join("kill.sh"))
        .env("QUECTO_CONTAINER_ROOT", root.path())
        .env("QUECTO_ENVIRONMENT_UUID", &create.environment_id)
        .env(
            "QUECTO_WORKSPACE_PATH",
            root.path().join("contract-agent/workspace"),
        )
        .output()
        .unwrap();
    assert!(kill.status.success());
    super::container_script_cleanup::validate_kill_output(&kill.stdout)
        .expect("kill fixture satisfies strict cleanup parser");

    let exec = run_script_json(
        &script_root.join("exec.sh").display().to_string(),
        None,
        Some("C1"),
        &agent,
    )
    .await
    .expect("exec fixture output parses as JSON");
    validate_script_result(&exec).expect("exec fixture satisfies strict launch parser");
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
fn cascade_cleanup_kills_once_when_all_colocated_members_removed() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("kills.log");
    let script = temp.path().join("kill.sh");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"$QUECTO_ENVIRONMENT_UUID\" >> {}\nprintf '{{\"environment_id\":\"%s\",\"status\":\"removed\",\"workspace_path\":\"/workspace/quecto\",\"container_ref\":\"C1\",\"metadata\":{{}},\"cleanup\":{{\"removed\":true}}}}\\n' \"$QUECTO_ENVIRONMENT_UUID\"\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let registry = super::subagent_registry::new_registry();
    let mut a =
        super::subagent_registry::SubagentEntry::new(std::path::PathBuf::from("/tmp/a.sock"), 1);
    a.environment_id = Some("env-1".into());
    a.container_kill_command = Some(script.display().to_string());
    let mut b =
        super::subagent_registry::SubagentEntry::new(std::path::PathBuf::from("/tmp/b.sock"), 2);
    b.environment_id = Some("env-1".into());
    b.container_kill_command = Some(script.display().to_string());
    let removed = vec![("agent-a".into(), a), ("agent-b".into(), b)];

    super::container_script_cleanup::cleanup_container_environments_after_removal(
        &removed, &registry, None,
    )
    .expect("cleanup succeeds");

    let kills = std::fs::read_to_string(&log).unwrap();
    assert_eq!(kills.lines().collect::<Vec<_>>(), vec!["env-1"]);
}

#[test]
fn cascade_cleanup_skips_kill_when_colocated_live_member_remains() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("kills.log");
    let script = temp.path().join("kill.sh");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"$QUECTO_ENVIRONMENT_UUID\" >> {}\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let registry = super::subagent_registry::new_registry();
    let mut live =
        super::subagent_registry::SubagentEntry::new(std::path::PathBuf::from("/tmp/b.sock"), 2);
    live.environment_id = Some("env-1".into());
    registry.lock().unwrap().insert("agent-b".into(), live);
    let mut removed_agent =
        super::subagent_registry::SubagentEntry::new(std::path::PathBuf::from("/tmp/a.sock"), 1);
    removed_agent.environment_id = Some("env-1".into());
    removed_agent.container_kill_command = Some(script.display().to_string());
    let removed = vec![("agent-a".into(), removed_agent)];

    super::container_script_cleanup::cleanup_container_environments_after_removal(
        &removed, &registry, None,
    )
    .expect("cleanup succeeds");

    assert!(
        !log.exists(),
        "kill script must not run while another live member remains"
    );
}
