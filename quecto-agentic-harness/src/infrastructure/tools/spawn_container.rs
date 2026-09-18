use serde::Deserialize;
use std::path::Path;

use crate::application::subagents::dto::{
    ContainerConfigSource, SelectContainerConfigRequest, SelectedContainerConfig,
};
use crate::application::subagents::use_cases::SelectContainerConfig;
use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentRegistry};
use crate::domain::error::DomainError;
use crate::domain::subagent::{ContainerSelection, SubagentConfig};
use crate::domain::subagent_launch::ParentEndpoint;
use crate::infrastructure::processes::containers::script_stderr::{
    ScriptOutput, ScriptStdout, run_capturing_stderr_tail,
};
use crate::infrastructure::processes::containers::standard::integrity::refuse_altered_script;
use crate::infrastructure::processes::owned_child_supervisor::{
    OwnedChildSupervisor, ProcessGroup, ProtocolOutcome, TerminationBudget,
};
use std::sync::Arc;

#[path = "spawn_prepared.rs"]
mod prepared;
pub(super) use prepared::{PreparedChild, run_cleanup_once};

/// The child command a launch adapter must run (or hand to a create script):
/// binary, final CLI args, and the parent's base directory.
pub(super) struct ChildCommand<'a> {
    pub swarm_context: Option<&'a super::swarm_bridge::SwarmContext>,
    /// The one owner of every locally spawned process (#1935).
    pub supervisor: &'a Arc<OwnedChildSupervisor>,
    pub binary: &'a Path,
    pub cli_args: &'a [std::ffi::OsString],
    pub base_dir: &'a Path,
    /// Client directory of the shared admission authority when this process
    /// is admission-enabled (#1679 P3). Script-managed runtimes must expose
    /// it to the child by path and report the capability, or the launch fails.
    pub admission_dir: Option<&'a Path>,
}

/// The only container admission capability P3 supports: the authority's
/// client directory is bind-mounted at the same path inside the container.
pub const ADMISSION_CAPABILITY_SHARED_DIRECTORY: &str = "shared-directory-v1";

fn require_admission_capability(
    admission_dir: Option<&Path>,
    reported: Option<&str>,
    operation: &str,
) -> Result<(), DomainError> {
    match (admission_dir, reported) {
        (None, _) => Ok(()),
        (Some(_), Some(ADMISSION_CAPABILITY_SHARED_DIRECTORY)) => Ok(()),
        (Some(dir), other) => Err(DomainError::Tool(format!(
            "script-managed {operation} did not report admission capability '{ADMISSION_CAPABILITY_SHARED_DIRECTORY}' for {} (reported {other:?}); admission-enabled launches never bypass the authority",
            dir.display()
        ))),
    }
}

/// `selection` is the composed container-config selection (#2024 S4a):
/// launch policy over the launching agent's effective configuration for
/// its checkout. A launcher composed without one refuses every new
/// container before any script runs.
pub(super) async fn spawn_prepared_child(
    config: &SubagentConfig,
    child: &ChildCommand<'_>,
    environments: &EnvironmentRegistry,
    selection: Option<&SelectContainerConfig>,
) -> Result<PreparedChild, DomainError> {
    if child.swarm_context.is_some() && !matches!(config.container, ContainerSelection::Local) {
        return Err(DomainError::Tool("swarm members must reuse their shared container and fixed pool; nested containers cannot reset admission".into()));
    }
    match &config.container {
        ContainerSelection::Local => spawn_local_child(child).await,
        ContainerSelection::New {
            container_config, ..
        } => {
            let selected = select_container_config(selection, config, container_config)?;
            spawn_script_managed_child(config, child, &selected, environments).await
        }
        ContainerSelection::Existing { target } => {
            join_script_managed_child(child, environments, target).await
        }
    }
}

pub(super) const NO_CONTAINER_CONFIG_SELECTION_COMPOSED: &str =
    "container spawn requires a composed container-config selection; this launcher has none";

/// The config a new container launches with: an explicit spawn `config`
/// argument replaces the launching agent's layers (as `--config` does);
/// otherwise the launching agent's own effective configuration — its base
/// file with its checkout's trusted overlay — supplies the entries. The
/// layer diagnostics (an untrusted or refused overlay that was not
/// applied) reach the operator's stderr, as every other load reports
/// them, and travel with the selection into the spawn result — or, when
/// the selection fails, inside the tool error's text.
fn select_container_config(
    selection: Option<&SelectContainerConfig>,
    config: &SubagentConfig,
    name: &Option<String>,
) -> Result<SelectedContainerConfig, DomainError> {
    let selection = selection
        .ok_or_else(|| DomainError::Tool(NO_CONTAINER_CONFIG_SELECTION_COMPOSED.into()))?;
    let source = match &config.config_path {
        Some(path) => ContainerConfigSource::Explicit(path.clone()),
        None => ContainerConfigSource::LaunchingAgent,
    };
    let selected = selection
        .execute(&SelectContainerConfigRequest {
            source,
            name: name.clone(),
        })
        .map_err(|error| {
            report_diagnostics(error.diagnostics());
            DomainError::Tool(error.to_string())
        })?;
    report_diagnostics(&selected.diagnostics);
    Ok(selected)
}

fn report_diagnostics(diagnostics: &[String]) {
    if !diagnostics.is_empty() {
        eprintln!("{}", diagnostics.join("\n"));
    }
}

/// Join an existing committed environment (#1369 slice 2): resolve the target
/// through the authoritative registry, then run the environment's *retained*
/// exec argv — never the currently configured script set — once its
/// program is judged intact (#2024 S4e).
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
    // The retained program is judged like the create's (#2024 S4e): a
    // standard-bundle script altered since the create never runs.
    if let Err(reason) = refuse_altered_script(&record.retained_exec_argv) {
        return Err(DomainError::Tool(format!(
            "join of environment {} refused: {reason}",
            record.environment_ref
        )));
    }
    let mut cmd = script_command(&record.retained_exec_argv, child.binary, child.cli_args);
    cmd.env("QUECTO_CONTAINER_CONFIG", &record.script_name);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_ID", &record.environment_id);
    apply_common_child_env(&mut cmd, child.base_dir);
    apply_admission_env(&mut cmd, child.admission_dir);
    let output = run_script(cmd, "exec").await?;
    let endpoint = parse_exec_result(&output.stdout, child.admission_dir)?;
    Ok(PreparedChild {
        swarm_reservation: None,
        owned_child: None,
        display_pid: 0,
        supervisor: Arc::clone(child.supervisor),
        environment_ref: Some(record.environment_ref),
        endpoint: Some(endpoint),
        proxy_bridge: None,
        process_owner: super::process_tree::ProcessOwner::DirectPid,
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        // Joining never owns the environment: a failed join must not
        // uncommit or stop it.
        environments: None,
        stderr_tail: None,
        container_diagnostics: Vec::new(),
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
    #[serde(default)]
    admission_capability: Option<String>,
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

fn parse_exec_result(
    stdout: &[u8],
    admission_dir: Option<&Path>,
) -> Result<ParentEndpoint, DomainError> {
    let wire: ExecResultWire = parse_strict_wire(stdout, "exec").map_err(DomainError::Tool)?;
    require_admission_capability(admission_dir, wire.admission_capability.as_deref(), "exec")?;
    endpoint_from_wire(wire.socket_path, wire.socket_proxy, &wire.metadata, "exec")
}

async fn spawn_local_child(child: &ChildCommand<'_>) -> Result<PreparedChild, DomainError> {
    let context = child.swarm_context.cloned();
    let mut reservation = tokio::task::spawn_blocking(move || {
        context
            .map(super::swarm_admission::LaunchReservation::reserve)
            .transpose()
    })
    .await
    .map_err(|e| DomainError::Tool(e.to_string()))??;
    let mut cmd = tokio::process::Command::new(child.binary);
    if let Some(reservation) = &reservation {
        reservation.configure(&mut cmd);
    }
    #[cfg(unix)]
    if reservation.is_some() {
        cmd.process_group(0);
    }
    cmd.args(child.cli_args);
    apply_common_child_env(&mut cmd, child.base_dir);
    // The child's stderr is drained for its whole life and its tail kept,
    // so a startup refusal reaches the launcher (#1937 review).
    cmd.stderr(std::process::Stdio::piped());
    let swarm_member = reservation.is_some();
    // Spawned and owned by the supervisor from the first instant: no other
    // holder of the process ever exists (#1935). The parent-loss contract
    // for an agent child is its launch-bound control connection, never a
    // parent-death signal: `PR_SET_PDEATHSIG` fires when the forking
    // *thread* exits, which a launcher on a short-lived runtime would
    // trigger spuriously.
    let spawned = child
        .supervisor
        .spawn(
            cmd,
            if swarm_member {
                ProcessGroup::Own
            } else {
                ProcessGroup::Inherited
            },
        )
        .await
        .map_err(|e| DomainError::Tool(format!("failed to spawn subagent: {e}")))?;
    let (handle, display_pid) = (spawned.handle, spawned.display_pid);
    let stderr_tail = spawned
        .stderr
        .map(|stderr| child.supervisor.retain_stderr_tail(stderr));
    if let Some(reservation) = &mut reservation {
        let result = if display_pid.0 == 0 {
            Err(DomainError::Tool("swarm child has no pid".into()))
        } else {
            reservation.launched(display_pid.0)
        };
        if let Err(error) = result {
            let outcome = child
                .supervisor
                .terminate(
                    handle,
                    async { ProtocolOutcome::Negative("swarm reservation refused".into()) },
                    TerminationBudget::DEFAULT,
                )
                .await;
            // Only an exit the owned handle observed confirms the member's
            // death (#1961); a child still running (or no longer retained)
            // keeps its reservation for reconciliation.
            use super::super::processes::owned_child_supervisor::TerminationOutcome;
            let exit = match outcome {
                TerminationOutcome::StillRunning { .. } | TerminationOutcome::NoRetainedHandle => {
                    None
                }
                TerminationOutcome::AlreadyExited(_) => {
                    Some(crate::domain::swarm::MemberExit::Abrupt)
                }
                TerminationOutcome::ExitedAfterProtocol(_)
                | TerminationOutcome::ExitedAfterTerm { .. }
                | TerminationOutcome::ExitedAfterKill { .. } => {
                    Some(crate::domain::swarm::MemberExit::Orderly)
                }
            };
            if let Some(exit) = exit {
                reservation.rolled_back(exit)?;
            }
            return Err(error);
        }
    }
    Ok(PreparedChild {
        swarm_reservation: reservation,
        owned_child: Some(handle),
        display_pid: display_pid.0,
        supervisor: Arc::clone(child.supervisor),
        environment_ref: None,
        endpoint: None,
        proxy_bridge: None,
        process_owner: if swarm_member {
            super::process_tree::ProcessOwner::LocalProcessGroup
        } else {
            super::process_tree::ProcessOwner::DirectPid
        },
        cleanup_environment_id: None,
        cleanup_argv: Vec::new(),
        environments: None,
        stderr_tail,
        container_diagnostics: Vec::new(),
    })
}

async fn spawn_script_managed_child(
    config: &SubagentConfig,
    child: &ChildCommand<'_>,
    selected: &SelectedContainerConfig,
    environments: &EnvironmentRegistry,
) -> Result<PreparedChild, DomainError> {
    let container = &selected.config;
    let config_name = container.name.as_str();
    let environment_ref = environments.mint_ref();
    let mut cmd = script_command(&container.create, child.binary, child.cli_args);
    cmd.env("QUECTO_CONTAINER_CONFIG", config_name);
    cmd.env("QUECTO_CONTAINER_ENVIRONMENT_REF", &environment_ref);
    apply_common_child_env(&mut cmd, child.base_dir);
    apply_admission_env(&mut cmd, child.admission_dir);
    let output = run_script(cmd, "create").await?;
    let result = match parse_create_result(&output.stdout, child.admission_dir) {
        Ok(result) => result,
        Err(e) => {
            let mut cleanup_argv = container.cleanup.clone();
            if let Some(env_id) = salvage_environment_id(&output.stdout) {
                run_cleanup_once(Some(env_id), &mut cleanup_argv).await;
            }
            return Err(e);
        }
    };
    if result
        .metadata
        .get(super::environment_commands::CHECKOUT_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .is_none_or(str::is_empty)
    {
        // Without it the host probes <workspace>/repo and <workspace> for a
        // coordination store before deciding whether a swarm's box must be
        // kept (#1924); say so once per create so a script author notices.
        tracing::warn!(
            environment_id = %result.environment_id,
            script = %config_name,
            "create result omits metadata.checkout; swarm retention will probe the workspace for the coordination store"
        );
    }
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
        swarm_reservation: None,
        owned_child: None,
        display_pid: 0,
        supervisor: Arc::clone(child.supervisor),
        environment_ref: Some(environment_ref),
        endpoint: Some(result.endpoint),
        proxy_bridge: None,
        process_owner: super::process_tree::ProcessOwner::DirectPid,
        cleanup_environment_id: Some(result.environment_id),
        cleanup_argv: container.cleanup.clone(),
        environments: Some(environments.clone()),
        stderr_tail: None,
        container_diagnostics: selected.diagnostics.clone(),
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
    #[serde(default)]
    admission_capability: Option<String>,
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

fn parse_create_result(
    stdout: &[u8],
    admission_dir: Option<&Path>,
) -> Result<CreateResult, DomainError> {
    let wire: CreateResultWire = parse_strict_wire(stdout, "create").map_err(DomainError::Tool)?;
    require_admission_capability(
        admission_dir,
        wire.admission_capability.as_deref(),
        "create",
    )?;
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

fn unsafe_arg(s: &str) -> bool {
    s.is_empty() || s.contains('\0')
}

/// Run a create or exec script to completion (#2024 S4b): its stdout is
/// the wire result, its stderr tail travels in the failure — after the
/// `script-managed <op> failed with status <n>` prefix — and is echoed on
/// the harness's stderr, so `die "image … is not present"` reaches both
/// the model and the operator instead of `/dev/null`.
async fn run_script(
    cmd: tokio::process::Command,
    operation: &str,
) -> Result<ScriptOutput, DomainError> {
    let output = run_capturing_stderr_tail(cmd, ScriptStdout::Result)
        .await
        .map_err(|e| {
            DomainError::Tool(format!("failed to invoke script-managed {operation}: {e}"))
        })?;
    if !output.status.success() {
        let message = output.failure_message(operation);
        eprintln!("{message}");
        return Err(DomainError::Tool(message));
    }
    Ok(output)
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
    Some(cmd)
}

fn apply_admission_env(cmd: &mut tokio::process::Command, admission_dir: Option<&Path>) {
    if let Some(dir) = admission_dir {
        cmd.env("QUECTO_ADMISSION_DIR", dir);
    }
}

/// The environment every child and script gets. Streams are the caller's
/// decision: a local child's stderr is drained for its whole life, a
/// script's is kept as a bounded tail for the failure report.
fn apply_common_child_env(cmd: &mut tokio::process::Command, base_dir: &Path) {
    if !base_dir.as_os_str().is_empty() {
        cmd.env("QUECTO_BASE_DIR", base_dir);
    }
    cmd.stdout(std::process::Stdio::null());
}

#[cfg(test)]
#[path = "spawn_container_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "spawn_container_owned_tests.rs"]
mod owned_tests;

#[cfg(test)]
#[path = "spawn_container_slice3_tests.rs"]
mod slice3_tests;

#[cfg(test)]
#[path = "spawn_container_admission_tests.rs"]
mod admission_tests;
