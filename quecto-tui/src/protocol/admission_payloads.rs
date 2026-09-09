//! Typed protocol values for the inference-admission view (#1679 P4): the
//! `admission` object of `get_state` and the `admission_state_changed` event.
//! Admission is never a lifecycle state; the view only adds a label beside
//! whatever phase the agent is in.

use serde::Deserialize;

/// A quota group's cooldown as last learned by the agent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionCooldown {
    /// `until`, `unknown` or `unavailable` (unknown strings are kept as-is).
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub remaining_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AdmissionGroupView {
    pub group: String,
    #[serde(default)]
    pub cooldown: Option<AdmissionCooldown>,
}

/// Bounded admission view of one agent process.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionView {
    #[serde(default)]
    pub waiting: u64,
    #[serde(default)]
    pub admitted: u64,
    #[serde(default)]
    pub longest_wait_seconds: Option<u64>,
    #[serde(default, deserialize_with = "lenient_groups")]
    pub groups: Vec<AdmissionGroupView>,
    #[serde(default)]
    pub revision: u64,
}

/// Groups are already bounded by the agent's configuration; this guards the
/// TUI against a hostile or buggy peer.
const MAX_GROUPS: usize = 32;
const MAX_GROUP_LABEL: usize = 48;

/// Malformed group entries are skipped rather than failing the whole view.
fn lenient_groups<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<AdmissionGroupView>, D::Error> {
    let raw: Vec<serde_json::Value> = Vec::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .take(MAX_GROUPS)
        .filter_map(|group| AdmissionGroupView::deserialize(group).ok())
        .collect())
}

/// Parse an `admission` value; `None` when absent, null or not an object.
pub fn parse_admission(
    value: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Option<AdmissionView> {
    if !value.is_object() {
        return None;
    }
    let mut view = AdmissionView::deserialize(value).ok()?;
    for group in &mut view.groups {
        group.group = sanitize(&group.group)
            .chars()
            .take(MAX_GROUP_LABEL)
            .collect();
        if let Some(cooldown) = &mut group.cooldown {
            cooldown.state = sanitize(&cooldown.state);
        }
    }
    Some(view)
}

impl AdmissionView {
    fn throttled(&self) -> Option<&AdmissionGroupView> {
        self.groups.iter().find(|g| {
            g.cooldown
                .as_ref()
                .is_some_and(|c| c.state != "until" || c.remaining_seconds.unwrap_or(0) > 0)
        })
    }

    /// A short status label, or `None` when nothing is worth showing: waiting
    /// attempts first (with the throttled group's cooldown when known), else
    /// an active dated cooldown, else nothing.
    pub fn status_label(&self) -> Option<String> {
        let throttled = self.throttled();
        if self.waiting > 0 {
            let mut label = match self.longest_wait_seconds {
                Some(seconds) => format!("waiting for admission {seconds}s"),
                None => "waiting for admission".to_string(),
            };
            if self.waiting > 1 {
                label.push_str(&format!(" (×{})", self.waiting));
            }
            if let Some(group) = throttled {
                label.push_str(&format!(" · {}", Self::cooldown_text(group)));
            }
            return Some(label);
        }
        throttled.map(Self::cooldown_text)
    }

    /// The panel-row form of [`Self::status_label`]: a few characters that
    /// survive a narrow sub-agent panel ("waiting 4s", "cooldown 30s").
    pub fn compact_label(&self) -> Option<String> {
        if self.waiting > 0 {
            return Some(match self.longest_wait_seconds {
                Some(seconds) => format!("waiting {seconds}s"),
                None => "waiting".to_string(),
            });
        }
        let cooldown = self.throttled()?.cooldown.as_ref()?;
        Some(match cooldown.state.as_str() {
            "until" => format!("cooldown {}s", cooldown.remaining_seconds.unwrap_or(0)),
            "unavailable" => "unavailable".to_string(),
            _ => "throttled".to_string(),
        })
    }

    fn cooldown_text(group: &AdmissionGroupView) -> String {
        match group.cooldown.as_ref() {
            Some(cooldown) if cooldown.state == "until" => format!(
                "{} cooldown {}s",
                group.group,
                cooldown.remaining_seconds.unwrap_or(0)
            ),
            Some(cooldown) if cooldown.state == "unavailable" => {
                format!("{} unavailable", group.group)
            }
            _ => format!("{} throttled", group.group),
        }
    }
}

#[cfg(test)]
#[path = "admission_payloads_tests.rs"]
mod tests;
