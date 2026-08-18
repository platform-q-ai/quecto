use std::path::{Path, PathBuf};

use crate::domain::ids::AgentUuid;
use crate::domain::subagent::SubagentConfig;

pub(super) fn inherited_runtime_config_path() -> Option<PathBuf> {
    std::env::var("QUECTO_RUNTIME_CONFIG_PATH")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub(super) fn effective_config_path(
    explicit_config_path: Option<&PathBuf>,
    inherited_config_path: Option<PathBuf>,
) -> Option<PathBuf> {
    explicit_config_path.cloned().or(inherited_config_path)
}

/// Durable child session key used for `-s` / `Session::build_key` (#1378).
/// Always the minted AgentUuid — never the user-facing display label.
pub(super) fn child_session_key(agent_uuid: &AgentUuid) -> &str {
    agent_uuid.as_str()
}

/// Socket path for a spawned child, keyed by AgentUuid (#1378).
pub(super) fn child_socket_path(socket_dir: &Path, agent_uuid: &AgentUuid) -> PathBuf {
    socket_dir.join(format!(
        "quecto-agent-{}.sock",
        child_session_key(agent_uuid)
    ))
}

/// Sidecar filename (workflow-spec / inherited policy) next to the socket.
pub(super) fn child_sidecar_filename(prefix: &str, agent_uuid: &AgentUuid, pid: u32) -> String {
    format!("{prefix}-{}-{pid}.json", child_session_key(agent_uuid))
}

use super::subagent_lifecycle::{SubagentLifecycleEvent, apply_lifecycle_event};
use super::subagent_registry::{ExitSignalTx, SubagentEntry};

pub(super) struct InitialRegistryEntrySpec<'a> {
    pub agent_uuid: AgentUuid,
    pub display_name: String,
    pub socket_path: PathBuf,
    pub pid: u32,
    pub parent_id: Option<String>,
    pub config: &'a SubagentConfig,
    pub exit_signal_tx: Option<ExitSignalTx>,
    pub cleanup_environment_id: Option<String>,
    pub cleanup_argv: Vec<String>,
    pub environment_registry: Option<crate::domain::environment_registry::EnvironmentRegistry>,
    pub environment_ref: Option<String>,
    pub process_owner: super::process_tree::ProcessOwner,
}

/// Build the registry entry used at spawn registration (production after socket
/// ready, and stub mode). Shared so the task-dependent initial status (#1049)
/// cannot drift between branches.
pub(super) fn initial_registry_entry(spec: InitialRegistryEntrySpec<'_>) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        spec.agent_uuid,
        spec.display_name,
        spec.socket_path,
        spec.pid,
    );
    entry.exit_signal_tx = spec.exit_signal_tx;
    entry.cleanup_environment_id = spec.cleanup_environment_id;
    entry.cleanup_argv = spec.cleanup_argv;
    entry.environment_registry = spec.environment_registry;
    entry.environment_ref = spec.environment_ref;
    entry.process_owner = spec.process_owner;
    // Stamp the child's parent as THIS agent's own id (#820 panel tree).
    entry.parent_id = spec.parent_id;
    // Record whether this child is a read-only observer (#966 / #957).
    entry.read_only = spec.config.read_only;
    if spec.config.task.is_none() {
        // #1049: task-less → Idle (cascade/TUI); with-task stays Starting.
        entry.status =
            apply_lifecycle_event(&mut entry.lifecycle, SubagentLifecycleEvent::RunEnded);
    }
    super::subagent_registry::seed_bound_workflow(&mut entry, spec.config.workflow_spec.as_ref());
    entry
}
