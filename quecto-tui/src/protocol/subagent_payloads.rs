//! Wire-format subagent roster payloads (#524/#525, #1369 slice 4).
//!
//! Split from `client.rs` (750-line cap): the typed `subagent_state_changed` /
//! `get_subagents` entry shape, including the additive versioned environment
//! metadata (`executionBackend`, `environment`) carried for script-managed
//! sub-agents. Additive camelCase fields with lenient defaults keep older
//! kernels parseable.

use serde::Deserialize;

/// Wire-format subagent info from `subagent_state_changed` events and
/// `get_subagents` responses (#524/#525).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentInfoEvent {
    /// Compatibility display label from legacy `agentId`.
    pub agent_id: String,
    #[serde(default)]
    pub agent_uuid: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    pub status: String,
    pub last_tool: Option<String>,
    pub last_error: Option<String>,
    pub pid: u32,
    /// Path to this sub-agent's own UDS socket, used to open a direct
    /// connect-on-select connection to its live stream (#800). `None` when the
    /// kernel did not surface it (older servers / non-local agents).
    #[serde(default)]
    pub socket_path: Option<String>,
    /// Spawning agent's id, for reconstructing the unit tree (PRD Stage B).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Latest workflow snapshot for this subagent, if any (PRD Stage B).
    #[serde(default)]
    pub workflow: Option<SubagentWorkflow>,
    /// Whether this sub-agent was spawned read-only (`write` + `edit` disabled).
    /// Drives the observer marker in the left panel (#966). Defaults to `false`
    /// for older kernels that did not surface the field.
    #[serde(default)]
    pub read_only: bool,
    /// How the sub-agent runs: `local` process or script-managed (`script`).
    /// Additive versioned field (#1369 slice 4); `None` on sparse refreshes and
    /// older kernels, preserved through sticky merge.
    #[serde(default)]
    pub execution_backend: Option<String>,
    /// Environment metadata for script-managed sub-agents; `None` for local
    /// ones. Additive versioned field (#1369 slice 4), preserved through
    /// sticky merge on sparse refreshes.
    #[serde(default)]
    pub environment: Option<SubagentEnvironmentInfo>,
}

/// Workflow snapshot mirror carried on a subagent entry (PRD Stage B).
/// Field names match the server's snake_case `WorkflowSnapshot` serialization.
#[derive(Debug, Clone, Deserialize)]
pub struct SubagentWorkflow {
    pub mode: String,
    pub steps_completed: u32,
    pub steps_total: u32,
}

/// Environment metadata carried on the subagent wire for script-managed
/// entries (#1369 slice 4). Every field defaults so a sparser producer cannot
/// fail the whole event parse.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEnvironmentInfo {
    /// Session-scoped `CN` ref minted by the harness environment registry.
    /// Display label only — refs restart at `C1` per session and can collide
    /// across forwarded descendant sessions; group on [`Self::group_key`].
    #[serde(rename = "ref", default)]
    pub environment_ref: String,
    /// Globally-unique environment identity (review #1392). Empty from older
    /// kernels, in which case grouping falls back to the session-scoped ref.
    #[serde(rename = "uuid", default)]
    pub environment_uuid: String,
    /// Optional user-facing environment name.
    #[serde(default)]
    pub name: Option<String>,
    /// Environment status label (`running`/`empty`/`killing`/`stopped`/
    /// `cleanup-failed`).
    #[serde(default)]
    pub status: String,
    /// Repository URL the environment was created for.
    #[serde(default)]
    pub repository: String,
    /// Branch recorded in the create-result metadata, when reported.
    #[serde(default)]
    pub branch: Option<String>,
    /// Script/runtime-owned environment identity from the create result.
    #[serde(default)]
    pub runtime_id: String,
    /// Workspace path shared by all member agents.
    #[serde(default)]
    pub workspace: String,
    /// How the parent reaches THIS member: `direct` UDS or a `proxy` bridge.
    /// Per-member, not environment-scoped (review #1392): one environment can
    /// mix modes, so environment chrome aggregates across members.
    #[serde(default)]
    pub socket_mode: String,
}

impl SubagentEnvironmentInfo {
    /// Grouping identity for this environment: the globally-unique uuid when
    /// the kernel reports one, else the session-scoped ref (older kernels).
    /// Session-scoped refs restart at `C1` per session, so grouping on the ref
    /// alone would merge unrelated environments forwarded from descendant
    /// sessions (review #1392).
    pub fn group_key(&self) -> &str {
        if self.environment_uuid.is_empty() {
            &self.environment_ref
        } else {
            &self.environment_uuid
        }
    }
}
