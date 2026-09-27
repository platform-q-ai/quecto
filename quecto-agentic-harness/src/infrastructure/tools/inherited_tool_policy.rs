use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolSource};
use crate::domain::tool_id::ToolIdentity;
use crate::infrastructure::tools::registration::ToolRegistration;

const INHERITED_TOOL_POLICY_SNAPSHOT_VERSION: u32 = 1;

pub(crate) use crate::infrastructure::tools::workflow_tool::{
    WORKFLOW_PROVIDER_ID, WORKFLOW_TOOL_NAME,
};

/// Entrypoint-only tools (#2216), as `(provider id, name)`: the bundled tools
/// that some entrypoints never build. A one-shot CLI agent has no workflow
/// runtime, so its snapshot holds no workflow entry; that absence is an
/// entrypoint difference, not a denial. An allowlist: add a tool here only
/// when not building it is never how a policy denies it.
const ENTRYPOINT_ONLY_TOOLS: &[(&str, &str)] = &[(WORKFLOW_PROVIDER_ID, WORKFLOW_TOOL_NAME)];

/// Each entrypoint-only tool's name and the registration it is built with.
pub(crate) fn entrypoint_only_tools() -> impl Iterator<Item = (&'static str, ToolRegistration)> {
    ENTRYPOINT_ONLY_TOOLS.iter().map(|(provider_id, name)| {
        (
            *name,
            ToolRegistration::official_native().with_provider_id(*provider_id),
        )
    })
}

/// Whether `name`, registered as `registration`, is an entrypoint-only tool:
/// the bundled registration itself, never a UDS or runtime tool that shares
/// its name or claims its stable id.
pub(crate) fn is_entrypoint_only(name: &str, registration: &ToolRegistration) -> bool {
    registration.source == ToolSource::BundledNative
        && registration.stable_id_override.is_none()
        && ENTRYPOINT_ONLY_TOOLS.iter().any(|(provider_id, tool)| {
            *tool == name && registration.provider_id.as_ref() == *provider_id
        })
}

/// The scope `tools` records for `name` with `identity`: by stable id, then
/// by name, the lookup a child applies.
pub(crate) fn recorded_scope(
    tools: &BTreeMap<String, ProfileAvailabilityScope>,
    name: &str,
    identity: &ToolIdentity,
) -> Option<ProfileAvailabilityScope> {
    tools
        .get(identity.stable_id.as_ref())
        .or_else(|| tools.get(name))
        .copied()
}

/// The bundled workflow tool's identity.
pub(crate) fn workflow_tool_identity() -> ToolIdentity {
    ToolIdentity::new(
        ToolSource::BundledNative,
        WORKFLOW_PROVIDER_ID,
        WORKFLOW_TOOL_NAME,
        Vec::new(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InheritedToolPolicySnapshot {
    pub version: u32,
    pub tools: BTreeMap<String, ProfileAvailabilityScope>,
}

impl InheritedToolPolicySnapshot {
    pub(super) fn new(tools: BTreeMap<String, ProfileAvailabilityScope>) -> Self {
        Self {
            version: INHERITED_TOOL_POLICY_SNAPSHOT_VERSION,
            tools,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != INHERITED_TOOL_POLICY_SNAPSHOT_VERSION {
            return Err(format!(
                "unsupported inherited tool policy snapshot version {}",
                self.version
            ));
        }
        if self.tools.keys().any(|name| name.trim().is_empty()) {
            return Err("inherited tool policy snapshot contains an empty tool id".into());
        }
        Ok(())
    }
}

pub(super) fn write_snapshot(
    path: &Path,
    snapshot: &InheritedToolPolicySnapshot,
) -> Result<(), String> {
    let data = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
    super::spawn_launch_args::write_private_new(path, &data).map_err(|e| e.to_string())
}

pub(crate) fn load_validate_unlink(path: &Path) -> Result<InheritedToolPolicySnapshot, String> {
    let data =
        std::fs::read(path).map_err(|e| format!("read inherited tool policy snapshot: {e}"))?;
    let _ = std::fs::remove_file(path);
    let snapshot: InheritedToolPolicySnapshot = serde_json::from_slice(&data)
        .map_err(|e| format!("parse inherited tool policy snapshot: {e}"))?;
    snapshot.validate()?;
    Ok(snapshot)
}
