use crate::domain::container_runtime::{
    ContainerScriptSet, ContainerScriptsConfig, ExistingContainerRef, SpawnContainerRequest,
};
use crate::domain::error::DomainError;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub trait AgentLaunchBackend: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn can_launch(&self, request: &SpawnContainerRequest) -> bool;
}

#[derive(Debug, Default)]
pub struct LocalProcessLaunchBackend;

impl AgentLaunchBackend for LocalProcessLaunchBackend {
    fn backend_name(&self) -> &'static str {
        "local"
    }
    fn can_launch(&self, request: &SpawnContainerRequest) -> bool {
        matches!(request, SpawnContainerRequest::Local)
    }
}

#[derive(Debug, Clone)]
pub struct ScriptManagedContainerLaunchBackend {
    config: ContainerScriptsConfig,
    parent_repo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerLaunchOutcome {
    pub environment_id: String,
    pub socket_path: PathBuf,
    pub workspace_path: PathBuf,
    pub metadata: Value,
    pub repository: Option<String>,
    pub script_name: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ContainerLaunchSpec<'a> {
    pub request: &'a SpawnContainerRequest,
    pub agent_uuid: &'a str,
    pub parent_agent_uuid: Option<&'a str>,
    pub child_args: &'a [String],
    pub requested_socket_path: &'a Path,
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ExistingContainerLaunchSpec<'a> {
    pub request: &'a SpawnContainerRequest,
    pub environment_id: &'a str,
    pub agent_uuid: &'a str,
    pub parent_agent_uuid: Option<&'a str>,
    pub child_args: &'a [String],
    pub requested_socket_path: &'a Path,
    pub read_only: bool,
}

impl Default for ScriptManagedContainerLaunchBackend {
    fn default() -> Self {
        Self::new(ContainerScriptsConfig::default(), None)
    }
}

impl ScriptManagedContainerLaunchBackend {
    pub fn new(config: ContainerScriptsConfig, parent_repo: Option<String>) -> Self {
        Self {
            config,
            parent_repo,
        }
    }

    pub fn resolve_new_request<'a>(
        &'a self,
        request: &'a SpawnContainerRequest,
    ) -> Result<(&'a str, &'a ContainerScriptSet, Option<String>), DomainError> {
        let Some((name, script)) = request
            .resolve_script(&self.config)
            .map_err(DomainError::Tool)?
        else {
            return Err(DomainError::Tool(
                "container backend requires a new-container request".into(),
            ));
        };
        let repo = match request {
            SpawnContainerRequest::New { repo, .. } => {
                repo.clone().or_else(|| self.parent_repo.clone())
            }
            _ => None,
        };
        Ok((name, script, repo))
    }

    pub async fn launch_new(
        &self,
        spec: &ContainerLaunchSpec<'_>,
    ) -> Result<ContainerLaunchOutcome, DomainError> {
        let (script_name, script, repo) = self.resolve_new_request(spec.request)?;
        let mut cmd = tokio::process::Command::new(&script.create);
        cmd.arg("--script-name")
            .arg(script_name)
            .arg("--socket-path")
            .arg(spec.requested_socket_path)
            .arg("--read-only")
            .arg(if spec.read_only { "true" } else { "false" });
        if let Some(repo) = &repo {
            cmd.arg("--repo").arg(repo);
            cmd.env("QUECTO_REPO_URL", repo);
        }
        cmd.arg("--").args(spec.child_args);
        cmd.env("QUECTO_AGENT_UUID", spec.agent_uuid);
        if let Some(parent) = spec.parent_agent_uuid {
            cmd.env("QUECTO_PARENT_AGENT_UUID", parent);
        }
        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .await
            .map_err(|e| {
                DomainError::Tool(format!(
                    "failed to run container create script '{script_name}': {e}"
                ))
            })?;
        if !output.status.success() {
            return Err(DomainError::Tool(format!(
                "container create script '{script_name}' failed with status {}",
                output.status
            )));
        }
        parse_launch_output(script_name, repo, &output.stdout)
    }

    pub async fn launch_existing(
        &self,
        spec: &ExistingContainerLaunchSpec<'_>,
    ) -> Result<ContainerLaunchOutcome, DomainError> {
        let SpawnContainerRequest::Existing { reference } = spec.request else {
            return Err(DomainError::Tool(
                "container exec requires an existing-container request".into(),
            ));
        };
        let (script_name, script) = SpawnContainerRequest::resolve_default_script(&self.config)
            .map_err(DomainError::Tool)?;
        let mut cmd = tokio::process::Command::new(&script.exec);
        cmd.arg("--script-name")
            .arg(script_name)
            .arg("--environment-id")
            .arg(spec.environment_id)
            .arg("--socket-path")
            .arg(spec.requested_socket_path)
            .arg("--read-only")
            .arg(if spec.read_only { "true" } else { "false" });
        match reference {
            ExistingContainerRef::Ref(r) => {
                cmd.arg("--container-ref")
                    .arg(r)
                    .env("QUECTO_CONTAINER_REF", r);
            }
            ExistingContainerRef::Name(n) => {
                cmd.arg("--container-name").arg(n);
            }
        };
        cmd.arg("--")
            .args(spec.child_args)
            .env("QUECTO_AGENT_UUID", spec.agent_uuid)
            .env("QUECTO_ENVIRONMENT_UUID", spec.environment_id);
        if let Some(parent) = spec.parent_agent_uuid {
            cmd.env("QUECTO_PARENT_AGENT_UUID", parent);
        }
        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .await
            .map_err(|e| {
                DomainError::Tool(format!(
                    "failed to run container exec script '{script_name}': {e}"
                ))
            })?;
        if !output.status.success() {
            return Err(DomainError::Tool(format!(
                "container exec script '{script_name}' failed with status {}",
                output.status
            )));
        }
        parse_launch_output(script_name, None, &output.stdout)
    }
}

fn parse_launch_output(
    script_name: &str,
    repo: Option<String>,
    stdout: &[u8],
) -> Result<ContainerLaunchOutcome, DomainError> {
    let value: Value = serde_json::from_slice(stdout).map_err(|e| {
        DomainError::Tool(format!(
            "container script '{script_name}' did not return JSON: {e}"
        ))
    })?;
    let str_field = |name: &str| -> Result<String, DomainError> {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                DomainError::Tool(format!(
                    "container script '{script_name}' output missing string field '{name}'"
                ))
            })
    };
    let socket = value
        .get("socket_path")
        .or_else(|| value.get("socket_proxy"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DomainError::Tool(format!(
                "container script '{script_name}' output missing socket_path or socket_proxy"
            ))
        })?;
    Ok(ContainerLaunchOutcome {
        environment_id: str_field("environment_id")?,
        socket_path: PathBuf::from(socket),
        workspace_path: PathBuf::from(str_field("workspace_path")?),
        metadata: value
            .get("metadata")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        repository: repo,
        script_name: script_name.to_string(),
    })
}

impl AgentLaunchBackend for ScriptManagedContainerLaunchBackend {
    fn backend_name(&self) -> &'static str {
        "container-script"
    }

    fn can_launch(&self, request: &SpawnContainerRequest) -> bool {
        matches!(
            request,
            SpawnContainerRequest::New { .. } | SpawnContainerRequest::Existing { .. }
        )
    }
}
