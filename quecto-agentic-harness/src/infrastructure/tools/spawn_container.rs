use serde::Deserialize;
use std::path::Path;

use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentRegistry};
use crate::domain::error::DomainError;
use crate::domain::subagent::{ContainerSelection, SubagentConfig};
use crate::domain::subagent_launch::ParentEndpoint;
use crate::infrastructure::config::{Config, ContainerConfig};

#[derive(Debug)]
pub(super) struct PreparedChild {
    pub child: Option<tokio::process::Child>,
    pub environment_ref: Option<String>,
    /// Typed parent endpoint from the create/exec result (#1369 slice 3).
    /// `None` for local children, whose requested socket path is authoritative.
    pub endpoint: Option<ParentEndpoint>,
    /// Proxy bridge materialized at readiness; carried so registration can
    /// take ownership and rollback can abort it.
    pub proxy_bridge: Option<super::spawn_proxy_bridge::ProxyBridge>,
    pub process_owner: super::process_tree::ProcessOwner,
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
        endpoint: Option<ParentEndpoint>,
    ) -> Self {
        Self {
            child,
            environment_ref,
            endpoint,
            proxy_bridge: None,
            process_owner: super::process_tree::ProcessOwner::DirectPid,
            cleanup_environment_id: None,
            cleanup_argv: vec![],
            environments: None,
        }
    }

    /// True when this launch created its environment (rather than joining an
    /// existing one) and therefore owns the record on rollback.
    pub fn owns_environment(&self) -> bool {
        self.environments.is_some()
    }

    pub fn cleanup_plan(&self) -> (Option<String>, Vec<String>) {
        (
            self.cleanup_environment_id.clone(),
            self.cleanup_argv.clone(),
        )
    }

    pub async fn rollback_once(&mut self) {
        if let Some(child) = &mut self.child {
            if let Some(pid) = child.id() {
                super::process_tree::terminate_owned_process_tree(pid, self.process_owner);
            } else {
                let _ = child.kill().await;
            }
            let _ = child.wait().await;
        }
        if let Some(bridge) = self.proxy_bridge.take() {
            bridge.teardown();
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
    parent_config_path: Option<&Path>,
) -> Result<PreparedChild, DomainError> {
    match &config.container {
        ContainerSelection::Local => spawn_local_child(child),
        ContainerSelection::New {
            container_config, ..
        } => {
            spawn_script_managed_child(
                config,
                child,
                &load_container_config(config, parent_config_path, child.base_dir)?,
                container_config,
                environments,
            )
            .await
        }
        ContainerSelection::Existing { target } => {
            join_script_managed_child(child, environments, target).await
        }
    }
}

/// Join an existing committed environment (#1369 slice 2): resolve the target
/// through the authoritative registry, then run the environment's *retained*
/// exec argv — never the currently configured script set.
async fn join_script_managed_child(
    child: &ChildCommand<'_>,
    environments: &EnvironmentRegistry,
    target: &crate::domain::environment_registry::EnvironmentTarget,
) -> Result<PreparedChild, DomainError> {
    let record = environments
        .resolve_joinable(target)
        .map_err(|e| DomainError::Tool(e.to_string()))?;
    if record.retained_exec_argv.is_empty() {
        return Err(DomainError::Tool(format!(
            "environment {} has no retained exec argv; its script set does not support joins",
            record.environment_ref
        )));
    }
    let mut cmd = script_command(&record.retained_exec_argv, child.binary, child.cli_args);
    cmd.env("QUECTO_CONTAINER_CONFIG", &record.script_name);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", &record.environment_id);
    apply_common_child_env(&mut cmd, child.base_dir);
    cmd.stdout(std::process::Stdio::piped());
    let output = cmd
        .output()
        .await
        .map_err(|e| DomainError::Tool(format!("failed to invoke script-managed exec: {e}")))?;
    if !output.status.success() {
        return Err(DomainError::Tool(format!(
            "script-managed exec failed with status {}",
            output.status
        )));
    }
    let endpoint = parse_exec_result(&output.stdout)?;
    Ok(PreparedChild {
        child: None,
        environment_ref: Some(record.environment_ref),
        endpoint: Some(endpoint),
        proxy_bridge: None,
        process_owner: super::process_tree::ProcessOwner::DirectPid,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        // Joining never owns the environment: a failed join must not
        // uncommit or stop it.
        environments: None,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecResultWire {
    metadata: serde_json::Value,
    #[serde(default)]
    socket_path: Option<std::path::PathBuf>,
    #[serde(default)]
    socket_proxy: Option<SocketProxyWire>,
}

/// Wire shape of a validated proxy endpoint (#1369 slice 3): an argv the
/// parent runs per connection as a stdio<->child bridge. Unknown keys (for
/// example a `shell` string) are rejected — argv-only, no interpolation.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SocketProxyWire {
    argv: Vec<String>,
}

/// Strict wire parse shared by the create, exec, and inspect result
/// contracts: UTF-8 only, exactly one JSON value, trailing data rejected.
/// Unknown-key rejection comes from each wire type's `deny_unknown_fields`.
/// Returns a plain error string so both launch-path (`DomainError`) and
/// post-mortem (`String`) callers share one definition.
pub(super) fn parse_strict_wire<T: serde::de::DeserializeOwned>(
    stdout: &[u8],
    operation: &str,
) -> Result<T, String> {
    let text = std::str::from_utf8(stdout)
        .map_err(|e| format!("script-managed {operation} returned non-UTF8 JSON: {e}"))?;
    let mut de = serde_json::Deserializer::from_str(text);
    let wire = T::deserialize(&mut de)
        .map_err(|e| format!("script-managed {operation} returned invalid JSON contract: {e}"))?;
    de.end()
        .map_err(|e| format!("script-managed {operation} returned extra JSON data: {e}"))?;
    Ok(wire)
}

/// Shared endpoint validation for the create and exec results (#1369 slice
/// 3): a metadata object plus EXACTLY ONE of a non-empty direct `socket_path`
/// or a validated `socket_proxy` argv.
fn endpoint_from_wire(
    socket_path: Option<std::path::PathBuf>,
    socket_proxy: Option<SocketProxyWire>,
    metadata: &serde_json::Value,
    operation: &str,
) -> Result<ParentEndpoint, DomainError> {
    if !metadata.is_object() {
        return Err(DomainError::Tool(format!(
            "script-managed {operation} result must contain a metadata object"
        )));
    }
    // A present-but-empty socket_path is still a PRESENT endpoint field: it
    // must fail the exactly-one check when socket_proxy is also carried (a
    // buggy direct-mode template), not silently collapse into proxy mode.
    match (socket_path, socket_proxy) {
        (Some(socket_path), None) => {
            if socket_path.as_os_str().is_empty() {
                return Err(DomainError::Tool(format!(
                    "script-managed {operation} result socket_path must be non-empty"
                )));
            }
            Ok(ParentEndpoint::Direct { socket_path })
        }
        (None, Some(proxy)) => {
            if proxy.argv.is_empty() || proxy.argv.iter().any(|s| unsafe_arg(s)) {
                return Err(DomainError::Tool(format!(
                    "script-managed {operation} result socket_proxy argv must be non-empty and safe"
                )));
            }
            Ok(ParentEndpoint::Proxy { argv: proxy.argv })
        }
        (Some(_), Some(_)) | (None, None) => Err(DomainError::Tool(format!(
            "script-managed {operation} result must carry exactly one of socket_path or socket_proxy"
        ))),
    }
}

fn parse_exec_result(stdout: &[u8]) -> Result<ParentEndpoint, DomainError> {
    let wire: ExecResultWire = parse_strict_wire(stdout, "exec").map_err(DomainError::Tool)?;
    endpoint_from_wire(wire.socket_path, wire.socket_proxy, &wire.metadata, "exec")
}

fn spawn_local_child(child: &ChildCommand<'_>) -> Result<PreparedChild, DomainError> {
    let mut cmd = tokio::process::Command::new(child.binary);
    cmd.args(child.cli_args);
    apply_common_child_env(&mut cmd, child.base_dir);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd
        .spawn()
        .map_err(|e| DomainError::Tool(format!("failed to spawn subagent: {e}")))?;
    Ok(PreparedChild {
        child: Some(child),
        environment_ref: None,
        endpoint: None,
        proxy_bridge: None,
        process_owner: super::process_tree::ProcessOwner::LocalProcessGroup,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        environments: None,
    })
}

/// Resolve the trusted config file `container_configs` loads from: an explicit
/// spawn `config` argument wins; without one the parent's own effective config
/// path is used (#1369 follow-up), so `container: true` works without the
/// caller hunting for the config location. Whichever path is chosen must be
/// absolute — the trusted-path requirement is not relaxed by the fallback.
fn container_config_load_error(e: crate::infrastructure::config::ConfigError) -> DomainError {
    match e {
        // ContainerConfigs errors already name the section — wrapping them
        // again would stutter ("invalid container_configs configuration:
        // invalid container_configs: ...").
        e @ crate::infrastructure::config::ConfigError::ContainerConfigs(_) => {
            DomainError::Tool(e.to_string())
        }
        e => DomainError::Tool(format!("invalid container_configs configuration: {e}")),
    }
}

pub(super) fn load_container_config(
    config: &SubagentConfig,
    parent_config_path: Option<&Path>,
    checkout: &Path,
) -> Result<Config, DomainError> {
    load_container_config_with_trust(config, parent_config_path, checkout, true)
}

pub(super) fn load_container_config_for_roster(
    config: &SubagentConfig,
    parent_config_path: Option<&Path>,
    checkout: &Path,
) -> Result<Config, DomainError> {
    load_container_config_with_trust(config, parent_config_path, checkout, false)
}

fn load_container_config_with_trust(
    config: &SubagentConfig,
    parent_config_path: Option<&Path>,
    checkout: &Path,
    prompt_on_miss: bool,
) -> Result<Config, DomainError> {
    let cfg_path = config
        .config_path
        .as_deref()
        .or(parent_config_path)
        .ok_or_else(|| {
            DomainError::Tool(
                "container spawn requires --config so container_configs can be loaded".into(),
            )
        })?;
    if !cfg_path.is_absolute() {
        return Err(DomainError::Tool(
            "container spawn requires an absolute trusted config path".into(),
        ));
    }
    let global = Config::load(&cfg_path.to_string_lossy()).map_err(container_config_load_error)?;
    let mut trust = if prompt_on_miss {
        crate::infrastructure::repo_local_container_config::PersistentRepoLocalContainerConfigTrust::new()
    } else {
        crate::infrastructure::repo_local_container_config::PersistentRepoLocalContainerConfigTrust::read_only()
    };
    let effective = crate::infrastructure::repo_local_container_config::effective_container_configs_for_checkout(
        global, checkout, &mut trust,
    )
    .map_err(container_config_load_error)?;
    if !effective.diagnostics.is_empty() {
        eprintln!("{}", effective.diagnostics.join("\n"));
    }
    Ok(effective.config)
}

async fn spawn_script_managed_child(
    config: &SubagentConfig,
    child: &ChildCommand<'_>,
    cfg: &Config,
    selected_config: &Option<String>,
    environments: &EnvironmentRegistry,
) -> Result<PreparedChild, DomainError> {
    let config_name = container_config_name(selected_config, cfg)?;
    let container = container_config(cfg, config_name)?;
    validate_container_config(container)?;
    let environment_ref = environments.mint_ref();
    let mut cmd = script_command(&container.create, child.binary, child.cli_args);
    cmd.env("QUECTO_CONTAINER_CONFIG", config_name);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_REF", &environment_ref);
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
            let mut cleanup_argv = container.cleanup.clone();
            if let Some(env_id) = salvage_environment_id(&output.stdout) {
                run_cleanup_once(Some(env_id), &mut cleanup_argv).await;
            }
            return Err(e);
        }
    };
    environments.commit(EnvironmentRecord {
        environment_ref: environment_ref.clone(),
        environment_id: result.environment_id.clone(),
        environment_uuid: crate::domain::environment_registry::mint_environment_uuid(),
        name: environment_name(config),
        workspace_path: result.workspace_path.clone(),
        // The config owns its source (#1410): the repository shown in
        // listings/TUI is whatever the create script truthfully reported in
        // its metadata; sandbox configs report none.
        repository: reported_repository(&result.metadata),
        script_name: config_name.to_string(),
        retained_exec_argv: container.exec.clone(),
        retained_kill_argv: container.kill.clone(),
        retained_cleanup_argv: container.cleanup.clone(),
        retained_inspect_argv: container.inspect.clone(),
        members: Vec::new(),
        status: crate::domain::environment_registry::EnvironmentStatus::Running,
        metadata: result.metadata.clone(),
        last_error: None,
    });
    Ok(PreparedChild {
        child: None,
        environment_ref: Some(environment_ref),
        endpoint: Some(result.endpoint),
        proxy_bridge: None,
        process_owner: super::process_tree::ProcessOwner::DirectPid,
        cleanup_environment_id: Some(result.environment_id),
        cleanup_argv: container.cleanup.clone(),
        environments: Some(environments.clone()),
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateResultWire {
    environment_id: String,
    workspace_path: std::path::PathBuf,
    metadata: serde_json::Value,
    #[serde(default)]
    socket_path: Option<std::path::PathBuf>,
    #[serde(default)]
    socket_proxy: Option<SocketProxyWire>,
}

#[derive(Debug)]
struct CreateResult {
    environment_id: String,
    workspace_path: std::path::PathBuf,
    endpoint: ParentEndpoint,
    metadata: serde_json::Value,
}

fn environment_name(config: &SubagentConfig) -> Option<String> {
    match &config.container {
        ContainerSelection::New { name, .. } => name.clone(),
        _ => None,
    }
}

/// Best-effort extraction of `environment_id` from a rejected create result so
/// the environment the script already created can still be cleaned up.
/// Deliberately permissive, unlike the strict `CreateResultWire` contract.
fn salvage_environment_id(stdout: &[u8]) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct SalvageWire {
        environment_id: String,
    }
    // Permissive on unknown keys AND trailing data: any rejected create result
    // whose leading object still names the environment must get cleanup.
    let text = std::str::from_utf8(stdout).ok()?;
    let mut de = serde_json::Deserializer::from_str(text);
    SalvageWire::deserialize(&mut de)
        .ok()
        .map(|wire| wire.environment_id)
        .filter(|id| !id.is_empty())
}

fn parse_create_result(stdout: &[u8]) -> Result<CreateResult, DomainError> {
    let wire: CreateResultWire = parse_strict_wire(stdout, "create").map_err(DomainError::Tool)?;
    if wire.environment_id.is_empty() || wire.workspace_path.as_os_str().is_empty() {
        return Err(DomainError::Tool(
            "script-managed create result must contain environment_id and workspace_path".into(),
        ));
    }
    let endpoint = endpoint_from_wire(
        wire.socket_path,
        wire.socket_proxy,
        &wire.metadata,
        "create",
    )?;
    Ok(CreateResult {
        environment_id: wire.environment_id,
        workspace_path: wire.workspace_path,
        endpoint,
        metadata: wire.metadata,
    })
}

/// Sorted names of every configured container config, for error messages and
/// the spawn tool's roster line: an agent that hits a selection error must be
/// able to offer the real menu instead of dead-ending (#1410).
pub(crate) fn container_config_names(cfg: &Config) -> Vec<String> {
    let mut names: Vec<String> = cfg.container_configs.keys().cloned().collect();
    names.sort_unstable();
    names
}

fn enumerate_names(cfg: &Config) -> String {
    let names = container_config_names(cfg);
    if names.is_empty() {
        "none configured".to_string()
    } else {
        names.join(", ")
    }
}

fn container_config_name<'a>(
    selected: &'a Option<String>,
    cfg: &'a Config,
) -> Result<&'a str, DomainError> {
    if let Some(name) = selected.as_deref() {
        return Ok(name);
    }
    // Config::load enforces exactly-one-default for non-empty maps, so the
    // only way to arrive here without one is an empty map: container spawning
    // was requested but no container configs are defined.
    let mut defaults: Vec<&str> = cfg
        .container_configs
        .iter()
        .filter(|(_, c)| c.default)
        .map(|(name, _)| name.as_str())
        .collect();
    defaults.sort_unstable();
    match defaults.as_slice() {
        [only] => Ok(only),
        _ => Err(DomainError::Tool(format!(
            "no container config is labeled \"default\": true (available container configs: {})",
            enumerate_names(cfg)
        ))),
    }
}

fn container_config<'a>(cfg: &'a Config, name: &str) -> Result<&'a ContainerConfig, DomainError> {
    cfg.container_configs.get(name).ok_or_else(|| {
        DomainError::Tool(format!(
            "unknown container config '{name}' (available container configs: {})",
            enumerate_names(cfg)
        ))
    })
}

/// The repository shown in listings and TUI chrome is whatever the create
/// script truthfully reported in its result metadata (the config owns its
/// source, #1410); sandbox configs report nothing and list as empty.
fn reported_repository(metadata: &serde_json::Value) -> String {
    metadata
        .get("repository")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Argv-safe script invocation: `<argv...> -- <child binary> <child args...>`.
/// Shared by the create and (retained) exec operations.
fn script_command(
    argv: &[String],
    binary: &Path,
    cli_args: &[std::ffi::OsString],
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.arg("--");
    cmd.arg(binary);
    cmd.args(cli_args);
    cmd
}

fn validate_container_config(container: &ContainerConfig) -> Result<(), DomainError> {
    if container.create.is_empty() {
        return Err(DomainError::Tool(
            "invalid container_configs configuration: missing create argv".into(),
        ));
    }
    if container.cleanup.is_empty() {
        return Err(DomainError::Tool(
            "invalid container_configs configuration: missing cleanup argv".into(),
        ));
    }
    if container
        .create
        .iter()
        .chain(container.cleanup.iter())
        .chain(container.exec.iter())
        .chain(container.kill.iter())
        .chain(container.inspect.iter())
        .any(|s| unsafe_arg(s))
    {
        return Err(DomainError::Tool(
            "invalid container_configs configuration: unsafe argv".into(),
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
    if let Some(env_id) = env_ref {
        cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", env_id);
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

#[cfg(test)]
#[path = "spawn_container_slice3_tests.rs"]
mod slice3_tests;
