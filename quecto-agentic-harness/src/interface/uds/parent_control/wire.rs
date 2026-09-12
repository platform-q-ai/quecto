//! Wire schema of the `bind_parent_control` presentation (#1935). Parsed,
//! never interpreted: the binding policy lives in `domain::parent_control`.
//!
//! The launcher's monitor (an outbound adapter) encodes the same shape; the
//! round-trip is pinned by the tests here.
use serde::{Deserialize, Serialize};

use crate::domain::parent_control::ParentControlCapability;
use crate::domain::subagent_teardown::LaunchGeneration;

/// Wire `type` of the presentation frame the parent sends first.
pub const BIND_PARENT_CONTROL: &str = "bind_parent_control";

/// The presentation frame: what the parent writes first on the connection
/// it binds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindParentControlWire {
    #[serde(rename = "type")]
    pub kind: String,
    pub generation: u64,
    pub capability: String,
}

impl BindParentControlWire {
    /// Cheap prefilter: a line that cannot be a presentation is never parsed.
    /// A JSON `\u00xx` escape could hide the type, so such lines are parsed.
    pub fn may_be_presentation(line: &str) -> bool {
        line.contains(BIND_PARENT_CONTROL) || line.contains("\\u00")
    }

    /// Claim: only a JSON object whose `type` is [`BIND_PARENT_CONTROL`] is a
    /// presentation. `Ok(None)` means "not a presentation, dispatch as
    /// usual"; `Err` means "claimed but malformed" (fail closed).
    pub fn claim(line: &str) -> Result<Option<Self>, String> {
        if !Self::may_be_presentation(line) {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let Ok(serde_json::Value::Object(object)) =
            serde_json::from_str::<serde_json::Value>(trimmed)
        else {
            return Ok(None);
        };
        if object.get("type").and_then(serde_json::Value::as_str) != Some(BIND_PARENT_CONTROL) {
            return Ok(None);
        }
        serde_json::from_str(trimmed)
            .map(Some)
            .map_err(|e| format!("malformed {BIND_PARENT_CONTROL}: {e}"))
    }

    /// The typed presentation, or why it is refused before comparison.
    pub fn presented(&self) -> Result<(LaunchGeneration, ParentControlCapability), String> {
        let capability =
            ParentControlCapability::parse(&self.capability).map_err(|e| e.to_string())?;
        Ok((LaunchGeneration::new(self.generation), capability))
    }
}

/// The child's acknowledgement of a successful binding (an ordinary
/// response line, so the listening parent can ignore it like any event).
pub fn bound_ack_line() -> String {
    format!("{{\"type\":\"response\",\"command\":\"{BIND_PARENT_CONTROL}\",\"success\":true}}\n")
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
