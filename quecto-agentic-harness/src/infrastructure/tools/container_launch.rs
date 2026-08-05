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
    pub is_new: bool,
}

#[derive(Debug, serde::Deserialize)]
struct ScriptResult {
    environment_id: String,
    #[serde(default)]
    workspace_path: Option<String>,
    #[serde(default)]
    socket_path: Option<String>,
    #[serde(default)]
    socket_proxy: Option<String>,
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
            let (name, set) = config
                .container
                .resolve_script(&ctx.scripts)
                .map_err(DomainError::Tool)?
                .ok_or_else(|| DomainError::Tool("container script selection missing".into()))?;
            let repo = repo.clone().or_else(|| ctx.parent_repo.clone());
            reject_unsafe_repo(repo.as_deref())?;
            let out = run_script_json(&set.create, repo.as_deref(), None, agent_uuid).await?;
            if let Some(proxy) = out.socket_proxy.as_deref() {
                crate::domain::agent_launch_backend::validate_socket_proxy(proxy, name)?;
            }
            let mut entry = entry_from_script(out, repo, agent_uuid.clone(), name, set);
            let registered = register_container(&ctx.registry, entry.clone());
            entry.container_ref = registered.container_ref.clone();
            Ok(Some(PreparedContainerLaunch {
                entry,
                is_new: true,
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
            let mut entry = ctx
                .registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entries
                .get(&uuid)
                .cloned()
                .ok_or_else(|| DomainError::Tool(format!("unknown container ref '{ref_text}'")))?;
            entry.agents.push(agent_uuid.clone());
            ctx.registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entries
                .insert(uuid, entry.clone());
            Ok(Some(PreparedContainerLaunch {
                entry,
                is_new: false,
            }))
        }
    }
}

pub struct ContainerExecSpec<'a> {
    pub entry: &'a ContainerEntry,
    pub agent_uuid: &'a AgentUuid,
    pub parent_id: Option<&'a str>,
    pub requested_socket_path: &'a std::path::Path,
    pub child_binary: &'a std::path::Path,
    pub child_args: &'a [std::ffi::OsString],
    pub prepend_child_binary: bool,
}

pub fn build_container_exec_command(
    spec: ContainerExecSpec<'_>,
) -> Result<tokio::process::Command, DomainError> {
    let mut c = command_from_config(&spec.entry.exec_command, "container exec")?;
    c.env("QUECTO_CONTAINER_REF", &spec.entry.container_ref);
    c.env("QUECTO_ENVIRONMENT_UUID", &spec.entry.environment_id);
    c.env("QUECTO_WORKSPACE_PATH", &spec.entry.workspace_path);
    c.env("QUECTO_AGENT_UUID", spec.agent_uuid.to_string());
    if let Some(parent) = spec.parent_id {
        c.env("QUECTO_PARENT_AGENT_UUID", parent);
    }
    if let Some(repo) = &spec.entry.repo_url {
        c.env("QUECTO_REPO_URL", repo);
    }
    c.env("QUECTO_SOCKET_PATH", spec.requested_socket_path.as_os_str());
    c.env("QUECTO_CHILD_BINARY", spec.child_binary);
    c.arg("--");
    if spec.prepend_child_binary {
        c.arg(spec.child_binary);
    }
    c.args(spec.child_args);
    Ok(c)
}

fn reject_unsafe_repo(repo: Option<&str>) -> Result<(), DomainError> {
    if let Some(repo) = repo {
        if repo.starts_with('-') || repo.chars().any(|c| c.is_control()) {
            return Err(DomainError::Tool(
                "repository URL must not start with '-' or contain control characters".into(),
            ));
        }
    }
    Ok(())
}

fn command_from_config(
    command: &str,
    context: &str,
) -> Result<tokio::process::Command, DomainError> {
    let argv = super::container_script_cleanup::parse_configured_script_command(command)
        .map_err(|e| DomainError::Tool(format!("{context} command is not argv-safe: {e}")))?;
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    Ok(cmd)
}

async fn run_script_json(
    command: &str,
    repo: Option<&str>,
    container_ref: Option<&str>,
    agent_uuid: &AgentUuid,
) -> Result<ScriptResult, DomainError> {
    let mut cmd = command_from_config(command, "container script")?;
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
    script_name: &str,
    set: &crate::domain::container_runtime::ContainerScriptSet,
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
        script_name: script_name.to_string(),
        exec_command: set.exec.clone(),
        inspect_command: set.inspect.clone(),
        kill_command: set.kill.clone(),
        socket_path: result.socket_path,
        socket_proxy: result.socket_proxy,
        metadata,
    }
}

pub fn default_parent_repo(base_dir: &std::path::Path) -> Option<String> {
    if base_dir.as_os_str().is_empty() {
        return None;
    }
    std::process::Command::new("git")
        .arg("-C")
        .arg(base_dir)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
