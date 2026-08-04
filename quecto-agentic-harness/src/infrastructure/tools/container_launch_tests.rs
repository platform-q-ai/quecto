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
    assert_eq!(launch.exec_command, "echo exec");
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
    assert_eq!(launch.exec_command, "echo reuse");
}
