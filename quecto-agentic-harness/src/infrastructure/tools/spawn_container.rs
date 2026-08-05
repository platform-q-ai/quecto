use serde::Deserialize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

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
}

impl PreparedChild {
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
    }
}

pub(super) async fn run_cleanup_once(env_ref: Option<String>, cleanup_argv: &mut Vec<String>) {
    if let Some(mut cmd) = cleanup_command(env_ref.as_deref(), cleanup_argv) {
        let _ = cmd.status().await;
        cleanup_argv.clear();
    }
}

pub(super) fn parse_container_selection(
    args: &serde_json::Value,
) -> Result<ContainerSelection, String> {
    let Some(value) = args.get("container") else {
        return Ok(ContainerSelection::Local);
    };
    parse_container_value(value)
}

fn parse_container_value(value: &serde_json::Value) -> Result<ContainerSelection, String> {
    match value {
        serde_json::Value::Bool(false) => Ok(ContainerSelection::Local),
        serde_json::Value::Bool(true) => Ok(ContainerSelection::New {
            repo: None,
            container_script: None,
        }),
        serde_json::Value::Object(map) => parse_container_object(map),
        _ => Err("container must be false, true, or an object".to_string()),
    }
}

fn parse_container_object(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<ContainerSelection, String> {
    reject_unknown_container_fields(map)?;
    require_new_mode(map)?;
    Ok(ContainerSelection::New {
        repo: optional_string(map, "repo")?,
        container_script: optional_string(map, "container_script")?,
    })
}

fn reject_unknown_container_fields(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let allowed = ["mode", "repo", "container_script"];
    if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("unknown container field '{key}'"));
    }
    Ok(())
}

fn require_new_mode(map: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    match map.get("mode").and_then(|v| v.as_str()) {
        Some("new") => Ok(()),
        Some("existing") => {
            Err("container mode 'existing' is not supported in this slice".to_string())
        }
        Some(other) => Err(format!("unsupported container mode '{other}'")),
        None => Err("container.mode is required".to_string()),
    }
}

fn optional_string(
    map: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, String> {
    map.get(key)
        .map(|v| {
            v.as_str()
                .ok_or_else(|| format!("container.{key} must be a string"))
        })
        .transpose()
        .map(|v| v.map(str::to_string))
}

pub(super) async fn spawn_prepared_child(
    config: &SubagentConfig,
    binary: &Path,
    cli_args: &[std::ffi::OsString],
    base_dir: &Path,
) -> Result<PreparedChild, DomainError> {
    match &config.container {
        ContainerSelection::Local => spawn_local_child(binary, cli_args, base_dir),
        ContainerSelection::New {
            container_script, ..
        } => {
            spawn_script_managed_child(
                config,
                binary,
                cli_args,
                base_dir,
                &load_container_config(config)?,
                container_script,
            )
            .await
        }
    }
}

fn spawn_local_child(
    binary: &Path,
    cli_args: &[std::ffi::OsString],
    base_dir: &Path,
) -> Result<PreparedChild, DomainError> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(cli_args);
    apply_common_child_env(&mut cmd, base_dir);
    let child = cmd
        .spawn()
        .map_err(|e| DomainError::Tool(format!("failed to spawn subagent: {e}")))?;
    Ok(PreparedChild {
        child: Some(child),
        environment_ref: None,
        socket_path: None,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
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
    binary: &Path,
    cli_args: &[std::ffi::OsString],
    base_dir: &Path,
    cfg: &Config,
    selected_script: &Option<String>,
) -> Result<PreparedChild, DomainError> {
    let script_name = script_name(selected_script, cfg)?;
    let script = script_config(cfg, script_name)?;
    validate_script(script)?;
    let mut cmd = create_command(script, binary, cli_args);
    set_script_env(&mut cmd, config, cfg, script_name, "");
    apply_common_child_env(&mut cmd, base_dir);
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
    Ok(PreparedChild {
        child: None,
        environment_ref: Some(new_environment_ref()),
        socket_path: Some(result.socket_path),
        cleanup_environment_id: Some(result.environment_id),
        cleanup_argv: script.cleanup.clone(),
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

static NEXT_ENVIRONMENT_REF: AtomicU64 = AtomicU64::new(1);

fn new_environment_ref() -> String {
    format!("C{}", NEXT_ENVIRONMENT_REF.fetch_add(1, Ordering::Relaxed))
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
    cfg: &Config,
    script_name: &str,
    env_ref: &str,
) {
    if let Some(repo) = selected_repo(config, cfg) {
        cmd.env("QUECTO_CONTAINER_REPO", repo);
    }
    cmd.env("QUECTO_CONTAINER_SCRIPT", script_name);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_REF", env_ref);
}

fn selected_repo(config: &SubagentConfig, cfg: &Config) -> Option<String> {
    match &config.container {
        ContainerSelection::New {
            repo: Some(repo), ..
        } => Some(repo.clone()),
        ContainerSelection::New { repo: None, .. } => cfg.agents.defaults.repo.clone(),
        ContainerSelection::Local => None,
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
