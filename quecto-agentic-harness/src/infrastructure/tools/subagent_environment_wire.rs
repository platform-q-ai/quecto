//! Environment metadata projection for the versioned subagent wire DTOs
//! (#1369 slice 4).
//!
//! The subagent wire contract (`subagent_state_changed` events and the
//! `get_subagents` snapshot `SubagentInfo`) is versioned by ADDITIVE camelCase
//! fields with lenient defaults — older consumers ignore unknown keys, newer
//! consumers default missing ones — so `executionBackend` and `environment`
//! extend it without a protocol version bump. This module owns the one
//! canonical projection from a registry entry to that wire shape, shared by
//! the live-event serializer (`subagent_cascade`), the snapshot builder
//! (`build_subagent_info_list`), and the forwarded-descendant merge
//! (`subagent_monitor_merge`) so the three paths can never drift.

use serde::{Deserialize, Serialize};

use super::subagent_registry::SubagentEntry;

/// Execution backend label for entries launched as plain local processes.
pub const BACKEND_LOCAL: &str = "local";
/// Execution backend label for script-managed (container) entries.
pub const BACKEND_SCRIPT: &str = "script";

/// Environment metadata carried on the subagent wire for script-managed
/// entries (#1369 slice 4). Additive and camelCase; every field defaults on
/// deserialization so sparser producers cannot fail the whole event parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEnvironmentWire {
    /// Session-scoped `CN` ref minted by the environment registry.
    #[serde(rename = "ref", default)]
    pub environment_ref: String,
    /// Optional user-facing environment name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Environment status label (`running`/`empty`/`killing`/`stopped`/
    /// `cleanup-failed`).
    #[serde(default)]
    pub status: String,
    /// Repository URL the environment was created for.
    #[serde(default)]
    pub repository: String,
    /// Branch recorded in the create-result metadata, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Script/runtime-owned environment identity from the create result.
    #[serde(default)]
    pub runtime_id: String,
    /// Workspace path shared by all member agents.
    #[serde(default)]
    pub workspace: String,
    /// How the parent reaches the child: `direct` UDS or a `proxy` bridge.
    #[serde(default)]
    pub socket_mode: String,
}

/// Project the environment wire object for `entry`, or `None` for local
/// entries.
///
/// Locally-launched script-managed entries derive it from the authoritative
/// session environment registry (so status/member changes are always
/// current); descendant entries merged from a child's forwarded snapshot
/// carry the child's reported object verbatim (`forwarded_environment`).
pub fn environment_wire(entry: &SubagentEntry) -> Option<SubagentEnvironmentWire> {
    if let Some(forwarded) = &entry.forwarded_environment {
        return Some(forwarded.clone());
    }
    let registry = entry.environment_registry.as_ref()?;
    let env_ref = entry.environment_ref.as_deref()?;
    let record = registry.get(env_ref)?;
    Some(SubagentEnvironmentWire {
        environment_ref: record.environment_ref.clone(),
        name: record.name.clone(),
        status: record.status_label().to_string(),
        repository: record.repository.clone(),
        branch: record
            .metadata
            .get("branch")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        runtime_id: record.environment_id.clone(),
        workspace: record.workspace_path.to_string_lossy().into_owned(),
        socket_mode: if entry.proxy_bridge_socket.is_some() {
            "proxy"
        } else {
            "direct"
        }
        .to_string(),
    })
}

/// Execution backend label for `entry`: the forwarded label when a child's
/// snapshot reported one, else `script` for registry-backed entries and
/// `local` for plain local processes.
pub fn execution_backend(
    entry: &SubagentEntry,
    environment: Option<&SubagentEnvironmentWire>,
) -> String {
    if let Some(backend) = &entry.forwarded_execution_backend {
        return backend.clone();
    }
    if environment.is_some() {
        BACKEND_SCRIPT.to_string()
    } else {
        BACKEND_LOCAL.to_string()
    }
}
