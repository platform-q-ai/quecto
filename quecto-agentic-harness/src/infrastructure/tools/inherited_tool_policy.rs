use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::domain::tool_descriptor::ProfileAvailabilityScope;

const INHERITED_TOOL_POLICY_SNAPSHOT_VERSION: u32 = 1;

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
