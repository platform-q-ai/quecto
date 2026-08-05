use std::path::Path;

use crate::domain::error::DomainError;
use crate::domain::subagent::{ContainerSelection, SubagentConfig};
use crate::infrastructure::config::{Config, ContainerScriptConfig};

#[derive(Debug)]
pub(super) struct PreparedChild {
    pub child: tokio::process::Child,
    pub environment_ref: Option<String>,
    cleanup_argv: Vec<String>,
}

impl PreparedChild {
    pub async fn rollback_once(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        if self.environment_ref.is_some() && !self.cleanup_argv.is_empty() {
            let mut cmd = tokio::process::Command::new(&self.cleanup_argv[0]);
            cmd.args(&self.cleanup_argv[1..]);
            if let Some(env_ref) = self.environment_ref.as_deref() {
                cmd.env("QUECTO_CONTAINER_ENVIRONMENT_REF", env_ref);
            }
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
            let _ = cmd.status().await;
            self.cleanup_argv.clear();
        }
    }
}

pub(super) fn parse_container_selection(
    args: &serde_json::Value,
) -> Result<ContainerSelection, String> {
    let Some(value) = args.get("container") else {
        return Ok(ContainerSelection::Local);
    };
    match value {
        serde_json::Value::Bool(false) => Ok(ContainerSelection::Local),
        serde_json::Value::Bool(true) => Ok(ContainerSelection::New {
            repo: None,
            container_script: None,
        }),
        serde_json::Value::Object(map) => {
            let allowed = ["mode", "repo", "container_script"];
            if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
                return Err(format!("unknown container field '{key}'"));
            }
            match map.get("mode").and_then(|v| v.as_str()) {
                Some("new") => {}
                Some("existing") => {
                    return Err(
                        "container mode 'existing' is not supported in this slice".to_string()
                    );
                }
                Some(other) => return Err(format!("unsupported container mode '{other}'")),
                None => return Err("container.mode is required".to_string()),
            }
            let repo = map
                .get("repo")
                .map(|v| v.as_str().ok_or("container.repo must be a string"))
                .transpose()?
                .map(str::to_string);
            let container_script = map
                .get("container_script")
                .map(|v| {
                    v.as_str()
                        .ok_or("container.container_script must be a string")
                })
                .transpose()?
                .map(str::to_string);
            Ok(ContainerSelection::New {
                repo,
                container_script,
            })
        }
        _ => Err("container must be false, true, or an object".to_string()),
    }
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
            let cfg = load_container_config(config)?;
            spawn_script_managed_child(config, binary, cli_args, base_dir, &cfg, container_script)
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
        child,
        environment_ref: None,
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

fn spawn_script_managed_child(
    config: &SubagentConfig,
    binary: &Path,
    cli_args: &[std::ffi::OsString],
    base_dir: &Path,
    cfg: &Config,
    container_script: &Option<String>,
) -> Result<PreparedChild, DomainError> {
    let script_name = container_script
        .as_deref()
        .unwrap_or(&cfg.container_scripts.default);
    if script_name.is_empty() {
        return Err(DomainError::Tool(
            "invalid container_scripts configuration: missing default".into(),
        ));
    }
    let script = cfg
        .container_scripts
        .scripts
        .get(script_name)
        .ok_or_else(|| {
            DomainError::Tool(format!(
                "invalid container_scripts configuration: script '{script_name}' not found"
            ))
        })?;
    validate_script(script)?;
    let env_ref = format!("C-{}", uuid::Uuid::new_v4());
    let mut cmd = tokio::process::Command::new(&script.create[0]);
    cmd.args(&script.create[1..]);
    cmd.arg("--");
    cmd.arg(binary);
    cmd.args(cli_args);
    if let Some(repo) = selected_repo(config, cfg) {
        cmd.env("QUECTO_CONTAINER_REPO", repo);
    }
    cmd.env("QUECTO_CONTAINER_SCRIPT", script_name);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_REF", &env_ref);
    apply_common_child_env(&mut cmd, base_dir);
    let child = cmd
        .spawn()
        .map_err(|e| DomainError::Tool(format!("failed to spawn script-managed subagent: {e}")))?;
    Ok(PreparedChild {
        child,
        environment_ref: Some(env_ref),
        cleanup_argv: script.cleanup.clone(),
    })
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
        .any(|s| s.is_empty() || s.contains('\0'))
    {
        return Err(DomainError::Tool(
            "invalid container_scripts configuration: unsafe argv".into(),
        ));
    }
    Ok(())
}

fn apply_common_child_env(cmd: &mut tokio::process::Command, base_dir: &Path) {
    if !base_dir.as_os_str().is_empty() {
        cmd.env("QUECTO_BASE_DIR", base_dir);
    }
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
}
