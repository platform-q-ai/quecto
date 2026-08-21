//! Wire-format subagent roster payloads (#524/#525, #1369 slice 4).
//!
//! Split from `client.rs` (750-line cap): the typed `subagent_state_changed` /
//! `get_subagents` entry shape, including the additive versioned environment
//! metadata (`executionBackend`, `environment`) carried for script-managed
//! sub-agents. Additive camelCase fields with lenient defaults keep older
//! kernels parseable.

use serde::{Deserialize, Deserializer};

/// Wire-format subagent info from `subagent_state_changed` events and
/// `get_subagents` responses (#524/#525).
#[derive(Debug, Clone)]
pub struct SubagentInfoEvent {
    /// Compatibility display label from legacy `agentId`.
    pub agent_id: String,
    pub agent_uuid: Option<String>,
    pub display_name: Option<String>,
    pub status: String,
    pub last_tool: Option<String>,
    pub last_error: Option<String>,
    /// True for sparse compact `get_subagents` DTO rows. Not wire-serialised;
    /// deserialization marks rows so sticky merge can distinguish absent compact
    /// fields from explicit full-event false/none.
    pub compact: bool,
    /// Compact `get_subagents` rows omit `pid`; treat that as unknown (0)
    /// instead of failing the whole roster parse and wiping the left panel.
    pub pid: u32,
    /// Path to this sub-agent's own UDS socket, used to open a direct
    /// connect-on-select connection to its live stream (#800). `None` when the
    /// kernel did not surface it (older servers / non-local agents).
    pub socket_path: Option<String>,
    /// Spawning agent's id, for reconstructing the unit tree (PRD Stage B).
    pub parent_id: Option<String>,
    /// Latest workflow snapshot for this subagent, if any (PRD Stage B).
    pub workflow: Option<SubagentWorkflow>,
    /// Whether this sub-agent was spawned read-only (`write` + `edit` disabled).
    /// Drives the observer marker in the left panel (#966). Defaults to `false`
    /// for older kernels that did not surface the field.
    pub read_only: bool,
    /// How the sub-agent runs: `local` process or script-managed (`script`).
    /// Additive versioned field (#1369 slice 4); `None` on sparse refreshes and
    /// older kernels, preserved through sticky merge.
    pub execution_backend: Option<String>,
    /// Environment metadata for script-managed sub-agents; `None` for local
    /// ones. Additive versioned field (#1369 slice 4), preserved through
    /// sticky merge on sparse refreshes.
    pub environment: Option<SubagentEnvironmentInfo>,
}

impl SubagentInfoEvent {
    pub fn is_compact(&self) -> bool {
        self.compact
    }
}

impl<'de> Deserialize<'de> for SubagentInfoEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            agent_id: String,
            #[serde(default)]
            agent_uuid: Option<String>,
            #[serde(default)]
            display_name: Option<String>,
            status: String,
            #[serde(default)]
            last_tool: Option<String>,
            #[serde(default)]
            last_error: Option<String>,
            #[serde(default)]
            pid: Option<u32>,
            #[serde(default)]
            socket_path: Option<String>,
            #[serde(default)]
            parent_id: Option<String>,
            #[serde(default)]
            workflow: Option<SubagentWorkflow>,
            #[serde(default)]
            read_only: Option<bool>,
            #[serde(default)]
            execution_backend: Option<String>,
            #[serde(
                default,
                alias = "environmentRef",
                deserialize_with = "deserialize_environment"
            )]
            environment: Option<SubagentEnvironmentInfo>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let compact = wire.pid.is_none()
            && wire.socket_path.is_none()
            && wire.parent_id.is_none()
            && wire.workflow.is_none()
            && wire.read_only.is_none()
            && wire.execution_backend.is_none()
            && wire.last_tool.is_none()
            && wire.last_error.is_none();
        Ok(Self {
            agent_id: wire.agent_id,
            agent_uuid: wire.agent_uuid,
            display_name: wire.display_name,
            status: wire.status,
            last_tool: wire.last_tool,
            last_error: wire.last_error,
            compact,
            pid: wire.pid.unwrap_or(0),
            socket_path: wire.socket_path,
            parent_id: wire.parent_id,
            workflow: wire.workflow,
            read_only: wire.read_only.unwrap_or(false),
            execution_backend: wire.execution_backend,
            environment: wire.environment,
        })
    }
}

fn deserialize_environment<'de, D>(
    deserializer: D,
) -> Result<Option<SubagentEnvironmentInfo>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum EnvironmentWire {
        Rich(SubagentEnvironmentInfo),
        Compact(String),
    }
    Ok(
        match Option::<EnvironmentWire>::deserialize(deserializer)? {
            Some(EnvironmentWire::Rich(info)) => Some(info),
            Some(EnvironmentWire::Compact(environment_ref)) if !environment_ref.is_empty() => {
                Some(SubagentEnvironmentInfo {
                    environment_ref,
                    ..SubagentEnvironmentInfo::default()
                })
            }
            Some(EnvironmentWire::Compact(_)) | None => None,
        },
    )
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
#[derive(Debug, Clone, Default, Deserialize)]
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
    ///
    /// Accepted compat degradation: when producers mix (one member reports a
    /// uuid, another's producer predates it), the two keys differ and one
    /// environment renders as separate solo rows with identical `CN` badges
    /// instead of one group. Display-only degradation — no crash, no
    /// misdirected commands — so no version negotiation is added.
    pub fn group_key(&self) -> &str {
        if self.environment_uuid.is_empty() {
            &self.environment_ref
        } else {
            &self.environment_uuid
        }
    }
}
