use serde::Deserialize;
use std::path::Path;

use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentRegistry};
use crate::domain::error::DomainError;
use crate::domain::subagent::{ContainerSelection, SubagentConfig};
use crate::infrastructure::config::{Config, ContainerScriptConfig};

#[derive(Debug)]
pub(super) struct PreparedChild {
    pub child: Option<tokio::process::Child>,
    pub environment_ref: Option<String>,
    pub socket_path: Option<std::path::PathBuf>,
    cleanup_environment_id: Option<String>,
    cleanup_argv: Vec<String>,
    /// Session registry the environment was committed to, so rollback can
    /// uncommit the entry it created.
    environments: Option<EnvironmentRegistry>,
}

impl PreparedChild {
    #[cfg(test)]
    pub(super) fn new_for_test(
        child: Option<tokio::process::Child>,
        environment_ref: Option<String>,
        socket_path: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            child,
            environment_ref,
            socket_path,
            cleanup_environment_id: None,
            cleanup_argv: vec![],
            environments: None,
        }
    }

    pub fn cleanup_plan(&self) -> (Option<String>, Vec<String>) {
        (
            self.cleanup_environment_id.clone(),
            self.cleanup_argv.clone(),
        )
    }

    pub async fn rollback_once(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        run_cleanup_once(self.cleanup_environment_id.clone(), &mut self.cleanup_argv).await;
        if let (Some(environments), Some(env_ref)) = (&self.environments, &self.environment_ref) {
            environments.remove(env_ref);
        }
    }
}

pub(super) async fn run_cleanup_once(env_ref: Option<String>, cleanup_argv: &mut Vec<String>) {
    if let Some(mut cmd) = cleanup_command(env_ref.as_deref(), cleanup_argv) {
        let _ = cmd.status().await;
        cleanup_argv.clear();
    }
}

/// The child command a launch adapter must run (or hand to a create script):
/// binary, final CLI args, and the parent's base directory.
pub(super) struct ChildCommand<'a> {
    pub binary: &'a Path,
    pub cli_args: &'a [std::ffi::OsString],
    pub base_dir: &'a Path,
}

pub(super) async fn spawn_prepared_child(
    config: &SubagentConfig,
    child: &ChildCommand<'_>,
    environments: &EnvironmentRegistry,
) -> Result<PreparedChild, DomainError> {
    match &config.container {
        ContainerSelection::Local => spawn_local_child(child),
        ContainerSelection::New {
            container_script, ..
        } => {
            spawn_script_managed_child(
                config,
                child,
                &load_container_config(config)?,
                container_script,
                environments,
            )
            .await
        }
    }
}

fn spawn_local_child(child: &ChildCommand<'_>) -> Result<PreparedChild, DomainError> {
    let mut cmd = tokio::process::Command::new(child.binary);
    cmd.args(child.cli_args);
    apply_common_child_env(&mut cmd, child.base_dir);
    let child = cmd
        .spawn()
        .map_err(|e| DomainError::Tool(format!("failed to spawn subagent: {e}")))?;
    Ok(PreparedChild {
        child: Some(child),
        environment_ref: None,
        socket_path: None,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        environments: None,
    })
}

fn load_container_config(config: &SubagentConfig) -> Result<Config, DomainError> {
    let cfg_path = config.config_path.as_ref().ok_or_else(|| {
        DomainError::Tool(
            "container spawn requires --config so container_scripts can be loaded".into(),
        )
    })?;
    if !cfg_path.is_absolute() {
        return Err(DomainError::Tool(
            "container spawn requires an absolute trusted config path".into(),
        ));
    }
    Config::load(&cfg_path.to_string_lossy())
        .map_err(|e| DomainError::Tool(format!("invalid container_scripts configuration: {e}")))
}

async fn spawn_script_managed_child(
    config: &SubagentConfig,
    child: &ChildCommand<'_>,
    cfg: &Config,
    selected_script: &Option<String>,
    environments: &EnvironmentRegistry,
) -> Result<PreparedChild, DomainError> {
    let script_name = script_name(selected_script, cfg)?;
    let script = script_config(cfg, script_name)?;
    validate_script(script)?;
    let environment_ref = environments.mint_ref();
    let mut cmd = create_command(script, child.binary, child.cli_args);
    set_script_env(
        &mut cmd,
        config,
        cfg,
        script_name,
        &environment_ref,
        child.base_dir,
    )?;
    apply_common_child_env(&mut cmd, child.base_dir);
    cmd.stdout(std::process::Stdio::piped());
    let output = cmd
        .output()
        .await
        .map_err(|e| DomainError::Tool(format!("failed to invoke script-managed create: {e}")))?;
    if !output.status.success() {
        return Err(DomainError::Tool(format!(
            "script-managed create failed with status {}",
            output.status
        )));
    }
    let result = match parse_create_result(&output.stdout) {
        Ok(result) => result,
        Err(e) => {
            let mut cleanup_argv = script.cleanup.clone();
            if let Ok(wire) = serde_json::from_slice::<CreateResultWire>(&output.stdout) {
                run_cleanup_once(Some(wire.environment_id), &mut cleanup_argv).await;
            }
            return Err(e);
        }
    };
    environments.commit(EnvironmentRecord {
        environment_ref: environment_ref.clone(),
        environment_id: result.environment_id.clone(),
        workspace_path: result.workspace_path.clone(),
        script_name: script_name.to_string(),
    });
    Ok(PreparedChild {
        child: None,
        environment_ref: Some(environment_ref),
        socket_path: Some(result.socket_path),
        cleanup_environment_id: Some(result.environment_id),
        cleanup_argv: script.cleanup.clone(),
        environments: Some(environments.clone()),
    })
}

#[derive(serde::Deserialize)]
struct CreateResultWire {
    environment_id: String,
    workspace_path: std::path::PathBuf,
    metadata: serde_json::Value,
    #[serde(default)]
    socket_path: std::path::PathBuf,
    #[serde(default)]
    socket_proxy: Option<serde_json::Value>,
}

struct CreateResult {
    environment_id: String,
    workspace_path: std::path::PathBuf,
    socket_path: std::path::PathBuf,
}

fn parse_create_result(stdout: &[u8]) -> Result<CreateResult, DomainError> {
    let text = std::str::from_utf8(stdout).map_err(|e| {
        DomainError::Tool(format!("script-managed create returned non-UTF8 JSON: {e}"))
    })?;
    let mut de = serde_json::Deserializer::from_str(text);
    let wire = CreateResultWire::deserialize(&mut de).map_err(|e| {
        DomainError::Tool(format!(
            "script-managed create returned invalid JSON contract: {e}"
        ))
    })?;
    de.end().map_err(|e| {
        DomainError::Tool(format!(
            "script-managed create returned extra JSON data: {e}"
        ))
    })?;
    if wire.environment_id.is_empty()
        || wire.workspace_path.as_os_str().is_empty()
        || wire.socket_path.as_os_str().is_empty()
        || wire.socket_proxy.is_some()
        || !wire.metadata.is_object()
    {
        return Err(DomainError::Tool("script-managed create result must contain environment_id, workspace_path, metadata object, and direct socket_path only".into()));
    }
    Ok(CreateResult {
        environment_id: wire.environment_id,
        workspace_path: wire.workspace_path,
        socket_path: wire.socket_path,
    })
}

fn script_name<'a>(selected: &'a Option<String>, cfg: &'a Config) -> Result<&'a str, DomainError> {
    let name = selected
        .as_deref()
        .unwrap_or(&cfg.container_scripts.default);
    if name.is_empty() {
        Err(DomainError::Tool(
            "invalid container_scripts configuration: missing default".into(),
        ))
    } else {
        Ok(name)
    }
}

fn script_config<'a>(
    cfg: &'a Config,
    name: &str,
) -> Result<&'a ContainerScriptConfig, DomainError> {
    cfg.container_scripts.scripts.get(name).ok_or_else(|| {
        DomainError::Tool(format!(
            "invalid container_scripts configuration: script '{name}' not found"
        ))
    })
}

fn create_command(
    script: &ContainerScriptConfig,
    binary: &Path,
    cli_args: &[std::ffi::OsString],
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(&script.create[0]);
    cmd.args(&script.create[1..]);
    cmd.arg("--");
    cmd.arg(binary);
    cmd.args(cli_args);
    cmd
}

fn set_script_env(
    cmd: &mut tokio::process::Command,
    config: &SubagentConfig,
    _cfg: &Config,
    script_name: &str,
    env_ref: &str,
    base_dir: &Path,
) -> Result<(), DomainError> {
    if let Some(repo) = selected_repo(config, base_dir)? {
        cmd.env("QUECTO_CONTAINER_REPO", repo);
    }
    cmd.env("QUECTO_CONTAINER_SCRIPT", script_name);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_REF", env_ref);
    Ok(())
}

fn selected_repo(config: &SubagentConfig, base_dir: &Path) -> Result<Option<String>, DomainError> {
    match &config.container {
        ContainerSelection::New {
            repo: Some(repo), ..
        } => Ok(Some(repo.clone())),
        ContainerSelection::New { repo: None, .. } => discover_parent_repo(base_dir).map(Some),
        ContainerSelection::Local => Ok(None),
    }
}

fn discover_parent_repo(base_dir: &Path) -> Result<String, DomainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(base_dir)
        .arg("config")
        .arg("--get")
        .arg("remote.origin.url")
        .output()
        .map_err(|e| {
            DomainError::Tool(format!(
                "failed to discover parent repository remote.origin.url: {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(DomainError::Tool(
            "container spawn requires parent checkout remote.origin.url when container.repo is omitted".into(),
        ));
    }
    let repo = String::from_utf8(output.stdout)
        .map_err(|e| {
            DomainError::Tool(format!(
                "parent repository remote.origin.url is not UTF-8: {e}"
            ))
        })?
        .trim()
        .to_string();
    if repo.is_empty() {
        Err(DomainError::Tool(
            "container spawn requires non-empty parent checkout remote.origin.url when container.repo is omitted".into(),
        ))
    } else {
        Ok(repo)
    }
}

fn validate_script(script: &ContainerScriptConfig) -> Result<(), DomainError> {
    if script.create.is_empty() {
        return Err(DomainError::Tool(
            "invalid container_scripts configuration: missing create argv".into(),
        ));
    }
    if script
        .create
        .iter()
        .chain(script.cleanup.iter())
        .any(|s| unsafe_arg(s))
    {
        return Err(DomainError::Tool(
            "invalid container_scripts configuration: unsafe argv".into(),
        ));
    }
    Ok(())
}

fn unsafe_arg(s: &str) -> bool {
    s.is_empty() || s.contains('\0')
}

fn cleanup_command(env_ref: Option<&str>, argv: &[String]) -> Option<tokio::process::Command> {
    if env_ref.is_none() || argv.is_empty() {
        return None;
    }
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if let Some(env_ref) = env_ref {
        cmd.env("QUECTO_CONTAINER_ENVIRONMENT_REF", env_ref);
    }
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    Some(cmd)
}

fn apply_common_child_env(cmd: &mut tokio::process::Command, base_dir: &Path) {
    if !base_dir.as_os_str().is_empty() {
        cmd.env("QUECTO_BASE_DIR", base_dir);
    }
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
}

#[cfg(test)]
#[path = "spawn_container_tests.rs"]
mod tests;
