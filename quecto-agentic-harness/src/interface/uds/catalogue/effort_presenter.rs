//! Wire rendering of reasoning-effort outcomes (#1848): the `set_effort`
//! reply and the `effort` / `effortLevels` pair every `get_state` shape
//! carries.

use crate::application::catalogue::dto::{EffortChangeError, EffortChangeOutcome};
use crate::domain::provider::EffortLevel;

/// The effort fields of a session-state snapshot: the level in effect and
/// the active model's vocabulary, as API strings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EffortStateView {
    pub effort: Option<String>,
    pub effort_levels: Vec<String>,
}

/// An effort string alone (no vocabulary known): the shape unit rigs hand a
/// state snapshot; production always supplies the full view.
impl From<Option<String>> for EffortStateView {
    fn from(effort: Option<String>) -> Self {
        Self {
            effort,
            effort_levels: Vec::new(),
        }
    }
}

impl EffortStateView {
    pub fn new(effort: Option<EffortLevel>, choices: &[EffortLevel]) -> Self {
        Self {
            effort: effort.map(|level| level.as_str().to_string()),
            effort_levels: choices
                .iter()
                .map(|level| level.as_str().to_string())
                .collect(),
        }
    }
}

/// The `set_effort` ok payload.
pub fn render_change(outcome: &EffortChangeOutcome) -> serde_json::Value {
    serde_json::json!({ "effort": outcome.effective.as_str() })
}

/// The `set_effort` error message.
pub fn render_error(error: &EffortChangeError) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "effort_presenter_tests.rs"]
mod tests;
