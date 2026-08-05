use super::agent_launch_backend::{
    AgentLaunchBackend, LocalProcessLaunchBackend, ScriptManagedContainerLaunchBackend,
};
use crate::domain::container_runtime::{ExistingContainerRef, SpawnContainerRequest};

#[test]
fn local_backend_accepts_only_local_requests() {
    let backend = LocalProcessLaunchBackend;
    assert_eq!(backend.backend_name(), "local");
    assert!(backend.can_launch(&SpawnContainerRequest::Local));
    assert!(!backend.can_launch(&SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    }));
    assert!(!backend.can_launch(&SpawnContainerRequest::Existing {
        reference: ExistingContainerRef::Ref("C1".into()),
    }));
    assert_eq!(backend.build_exec_command(), None);
}

#[test]
fn script_backend_exposes_launch_seam_for_container_exec() {
    let backend = ScriptManagedContainerLaunchBackend::default();
    assert_eq!(backend.backend_name(), "container-script");
    assert!(backend.can_launch(&SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    }));
    assert_eq!(
        backend.build_exec_command(),
        Some("script-managed-container")
    );
}

#[tokio::test]
async fn existing_container_exec_uses_retained_script_not_current_default() {
    use crate::domain::agent_launch_backend::{
        AgentLaunchBackend, AgentLaunchSpec, RetainedContainerScript,
        ScriptManagedContainerLaunchBackend,
    };
    use crate::domain::container_runtime::{
        ContainerScriptSet, ContainerScriptsConfig, ExistingContainerRef, SpawnContainerRequest,
    };
    use crate::infrastructure::tools::container_registry::{ContainerEntry, ContainerStatus};
    use std::collections::HashMap;
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("exec.log");
    let exec_a = dir.path().join("exec-a.py");
    let exec_b = dir.path().join("exec-b.py");
    for (path, label) in [(&exec_a, "A"), (&exec_b, "B")] {
        std::fs::write(path, format!("#!/usr/bin/env python3\nimport json\nopen({:?}, 'a').write('{}')\nprint(json.dumps({{'environment_id':'env-retained','workspace_path':'/workspace/a','metadata':{{}},'container_ref':'C1','socket_path':'{}'}}))\n", log, label, dir.path().join(format!("{label}.sock")).display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    let backend = ScriptManagedContainerLaunchBackend::new(
        ContainerScriptsConfig {
            default: Some("b".into()),
            scripts: HashMap::from([(
                "b".into(),
                ContainerScriptSet {
                    create: String::new(),
                    exec: exec_b.display().to_string(),
                    inspect: "inspect-b".into(),
                    kill: "kill-b".into(),
                },
            )]),
        },
        None,
    );
    let retained = ContainerEntry {
        container_uuid: "uuid-retained".into(),
        container_ref: "C1".into(),
        container_name: Some("dev-a".into()),
        environment_id: "env-retained".into(),
        repo_url: None,
        workspace_path: "/workspace/a".into(),
        status: ContainerStatus::Running,
        agents: vec![],
        script_name: "a".into(),
        exec_command: exec_a.display().to_string(),
        inspect_command: "inspect-a".into(),
        kill_command: "kill-a".into(),
        socket_path: None,
        socket_proxy: None,
        metadata: serde_json::json!({}),
        last_error: None,
    };
    let child = std::env::current_exe().unwrap();
    let request = SpawnContainerRequest::Existing {
        reference: ExistingContainerRef::Ref("C1".into()),
    };
    let prepared = backend
        .prepare_launch(AgentLaunchSpec {
            request: &request,
            agent_uuid: "agent-2",
            parent_agent_uuid: None,
            child_binary: &child,
            child_args: &[],
            requested_socket_path: &dir.path().join("requested.sock"),
            read_only: false,
            existing_environment_id: Some("env-retained"),
            retained_container_script: Some(RetainedContainerScript {
                environment_id: &retained.environment_id,
                script_name: &retained.script_name,
                exec_command: &retained.exec_command,
                inspect_command: &retained.inspect_command,
                kill_command: &retained.kill_command,
            }),
        })
        .await
        .unwrap();
    let launch = prepared.container.unwrap();
    assert_eq!(std::fs::read_to_string(log).unwrap(), "A");
    assert_eq!(launch.script_name, "a");
    assert_eq!(launch.exec_command, exec_a.display().to_string());
}

#[tokio::test]
async fn configured_container_command_rejects_shell_metacharacters() {
    use crate::domain::agent_launch_backend::{
        AgentLaunchBackend, AgentLaunchSpec, ScriptManagedContainerLaunchBackend,
    };
    use crate::domain::container_runtime::{
        ContainerScriptSet, ContainerScriptsConfig, SpawnContainerRequest,
    };
    use std::collections::HashMap;
    let backend = ScriptManagedContainerLaunchBackend::new(
        ContainerScriptsConfig {
            default: Some("bad".into()),
            scripts: HashMap::from([(
                "bad".into(),
                ContainerScriptSet {
                    create: "echo ok; rm -rf /".into(),
                    exec: "true".into(),
                    inspect: "true".into(),
                    kill: "true".into(),
                },
            )]),
        },
        None,
    );
    let dir = tempfile::tempdir().unwrap();
    let request = SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    };
    let err = backend
        .prepare_launch(AgentLaunchSpec {
            request: &request,
            agent_uuid: "agent-bad",
            parent_agent_uuid: None,
            child_binary: &std::env::current_exe().unwrap(),
            child_args: &[],
            requested_socket_path: &dir.path().join("bad.sock"),
            read_only: false,
            existing_environment_id: None,
            retained_container_script: None,
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("metacharacter"), "{err}");
}
