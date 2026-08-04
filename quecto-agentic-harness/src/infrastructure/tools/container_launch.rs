use crate::domain::container_runtime::{
    ContainerScriptsConfig, ExistingContainerRef, SpawnContainerRequest,
};
use crate::domain::error::DomainError;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent::SubagentConfig;

use super::container_registry::{
    ContainerEntry, ContainerRegistry, ContainerStatus, register_container, resolve_live_ref,
};

fn resolve_live_name(registry: &ContainerRegistry, name: &str) -> Result<String, String> {
    let state = registry.lock().unwrap_or_else(|e| e.into_inner());
    let matches: Vec<_> = state
        .entries
        .values()
        .filter(|entry| entry.container_name.as_deref() == Some(name))
        .collect();
    match matches.as_slice() {
        [] => Err(format!("unknown container name '{name}'")),
        [entry] if entry.status == ContainerStatus::Running => Ok(entry.container_uuid.clone()),
        [_] => Err(format!("container name '{name}' is not live")),
        _ => Err(format!("container name '{name}' is ambiguous")),
    }
}

#[derive(Debug, Clone)]
pub struct ContainerLaunchContext {
    pub registry: ContainerRegistry,
    pub scripts: ContainerScriptsConfig,
    pub parent_repo: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedContainerLaunch {
    pub entry: ContainerEntry,
    pub exec_command: String,
}

#[derive(Debug, serde::Deserialize)]
struct ScriptResult {
    environment_id: String,
    #[serde(default)]
    workspace_path: Option<String>,
    #[serde(default)]
    container_ref: Option<String>,
    #[serde(default)]
    container_id: Option<String>,
    #[serde(default)]
    container_name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    health: Option<String>,
    #[serde(default)]
    metadata: serde_json::Value,
}

pub fn validate_container_request(
    ctx: &ContainerLaunchContext,
    config: &SubagentConfig,
) -> Result<(), String> {
    match &config.container {
        SpawnContainerRequest::Local => Ok(()),
        SpawnContainerRequest::New { .. } => {
            config.container.resolve_script(&ctx.scripts).map(|_| ())
        }
        SpawnContainerRequest::Existing { .. } => {
            SpawnContainerRequest::resolve_default_script(&ctx.scripts).map(|_| ())
        }
    }
}

pub async fn prepare_container_launch(
    ctx: &ContainerLaunchContext,
    config: &SubagentConfig,
    agent_uuid: &AgentUuid,
) -> Result<Option<PreparedContainerLaunch>, DomainError> {
    match &config.container {
        SpawnContainerRequest::Local => Ok(None),
        SpawnContainerRequest::New { repo, .. } => {
            let (_name, set) = config
                .container
                .resolve_script(&ctx.scripts)
                .map_err(DomainError::Tool)?
                .ok_or_else(|| DomainError::Tool("container script selection missing".into()))?;
            let repo = repo.clone().or_else(|| ctx.parent_repo.clone());
            let out = run_script_json(&set.create, repo.as_deref(), None, agent_uuid).await?;
            let mut entry = entry_from_script(out, repo, agent_uuid.clone());
            let registered = register_container(&ctx.registry, entry.clone());
            entry.container_ref = registered.container_ref.clone();
            Ok(Some(PreparedContainerLaunch {
                entry,
                exec_command: set.exec.clone(),
            }))
        }
        SpawnContainerRequest::Existing { reference } => {
            let ref_text = match reference {
                ExistingContainerRef::Ref(r) | ExistingContainerRef::Name(r) => r.as_str(),
            };
            let uuid = match reference {
                ExistingContainerRef::Ref(_) => {
                    resolve_live_ref(&ctx.registry, ref_text).map_err(DomainError::Tool)?
                }
                ExistingContainerRef::Name(_) => {
                    resolve_live_name(&ctx.registry, ref_text).map_err(DomainError::Tool)?
                }
            };
            let entry = ctx
                .registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entries
                .get(&uuid)
                .cloned()
                .ok_or_else(|| DomainError::Tool(format!("unknown container ref '{ref_text}'")))?;
            let (_name, set) = SpawnContainerRequest::resolve_default_script(&ctx.scripts)
                .map_err(DomainError::Tool)?;
            Ok(Some(PreparedContainerLaunch {
                entry,
                exec_command: set.exec.clone(),
            }))
        }
    }
}

pub async fn exec_container_command(
    command: &str,
    entry: &ContainerEntry,
    agent_uuid: &AgentUuid,
) -> Result<(), DomainError> {
    let _ = run_script_json(
        command,
        entry.repo_url.as_deref(),
        Some(&entry.container_ref),
        agent_uuid,
    )
    .await?;
    Ok(())
}

async fn run_script_json(
    command: &str,
    repo: Option<&str>,
    container_ref: Option<&str>,
    agent_uuid: &AgentUuid,
) -> Result<ScriptResult, DomainError> {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.env("QUECTO_AGENT_UUID", agent_uuid.to_string());
    cmd.env("QUECTO_ENVIRONMENT_UUID", agent_uuid.to_string());
    if let Some(repo) = repo {
        cmd.env("QUECTO_REPO_URL", repo);
    }
    if let Some(reference) = container_ref {
        cmd.env("QUECTO_CONTAINER_REF", reference);
    }
    let out = cmd
        .output()
        .await
        .map_err(|e| DomainError::Tool(format!("failed to run container script: {e}")))?;
    if !out.status.success() {
        return Err(DomainError::Tool(format!(
            "container script failed with status {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| DomainError::Tool(format!("container script did not emit valid JSON: {e}")))
}

fn entry_from_script(
    result: ScriptResult,
    repo: Option<String>,
    agent_uuid: AgentUuid,
) -> ContainerEntry {
    let status = match result.status.as_deref() {
        Some("stopped") => ContainerStatus::Stopped,
        Some("unhealthy") => ContainerStatus::Unhealthy,
        _ => ContainerStatus::Running,
    };
    let mut metadata = result.metadata;
    if let Some(health) = result.health {
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert("health".into(), serde_json::Value::String(health));
        }
    }
    ContainerEntry {
        container_uuid: result
            .container_id
            .clone()
            .unwrap_or_else(|| result.environment_id.clone()),
        container_ref: result.container_ref.unwrap_or_default(),
        container_name: result.container_name,
        environment_id: result.environment_id,
        repo_url: repo,
        workspace_path: result
            .workspace_path
            .unwrap_or_else(|| "/workspace/quecto".into()),
        status,
        agents: vec![agent_uuid],
        metadata,
    }
}

pub fn default_parent_repo(base_dir: &std::path::Path) -> Option<String> {
    if base_dir.as_os_str().is_empty() {
        None
    } else {
        Some(base_dir.to_string_lossy().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::container_runtime::{ContainerScriptSet, ContainerScriptsConfig};
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
            registry: super::super::container_registry::new_container_registry(),
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
            registry: super::super::container_registry::new_container_registry(),
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
            registry: super::super::container_registry::new_container_registry(),
            scripts: script_config(
                "printf '%s' '{\"environment_id\":\"env-1\"}'".into(),
                "echo reuse".into(),
            ),
            parent_repo: None,
        };
        super::super::container_registry::register_container(
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
}
