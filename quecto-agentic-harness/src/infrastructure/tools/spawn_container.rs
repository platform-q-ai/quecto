use std::path::Path;

use crate::domain::error::DomainError;
use crate::domain::subagent::{ContainerSelection, SubagentConfig};

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
) -> Result<tokio::process::Child, DomainError> {
    match &config.container {
        ContainerSelection::Local => spawn_local_child(binary, cli_args, base_dir),
        ContainerSelection::New {
            container_script, ..
        } => spawn_script_managed_child(config, binary, cli_args, base_dir, container_script),
    }
}

fn spawn_local_child(
    binary: &Path,
    cli_args: &[std::ffi::OsString],
    base_dir: &Path,
) -> Result<tokio::process::Child, DomainError> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(cli_args);
    apply_common_child_env(&mut cmd, base_dir);
    cmd.spawn()
        .map_err(|e| DomainError::Tool(format!("failed to spawn subagent: {e}")))
}

fn spawn_script_managed_child(
    config: &SubagentConfig,
    binary: &Path,
    cli_args: &[std::ffi::OsString],
    base_dir: &Path,
    container_script: &Option<String>,
) -> Result<tokio::process::Child, DomainError> {
    let cfg_path = config.config_path.as_ref().ok_or_else(|| {
        DomainError::Tool(
            "container spawn requires --config so container_scripts can be loaded".into(),
        )
    })?;
    let cfg = crate::infrastructure::config::Config::load(&cfg_path.to_string_lossy())
        .map_err(|e| DomainError::Tool(format!("invalid container_scripts configuration: {e}")))?;
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
    if script.create.is_empty() {
        return Err(DomainError::Tool(
            "invalid container_scripts configuration: missing create argv".into(),
        ));
    }
    if script
        .create
        .iter()
        .any(|s| s.is_empty() || s.contains('\0'))
    {
        return Err(DomainError::Tool(
            "invalid container_scripts configuration: unsafe create argv".into(),
        ));
    }

    let mut cmd = tokio::process::Command::new(&script.create[0]);
    cmd.args(&script.create[1..]);
    cmd.arg("--");
    cmd.arg(binary);
    cmd.args(cli_args);
    if let ContainerSelection::New {
        repo: Some(repo), ..
    } = &config.container
    {
        cmd.env("QUECTO_CONTAINER_REPO", repo);
    }
    cmd.env("QUECTO_CONTAINER_SCRIPT", script_name);
    apply_common_child_env(&mut cmd, base_dir);
    cmd.spawn()
        .map_err(|e| DomainError::Tool(format!("failed to spawn script-managed subagent: {e}")))
}

fn apply_common_child_env(cmd: &mut tokio::process::Command, base_dir: &Path) {
    if !base_dir.as_os_str().is_empty() {
        cmd.env("QUECTO_BASE_DIR", base_dir);
    }
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
}
