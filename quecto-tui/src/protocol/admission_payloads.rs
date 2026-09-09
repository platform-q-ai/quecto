//! Typed protocol values for the inference-admission view (#1679 P4): the
//! `admission` object of `get_state` and the `admission_state_changed` event.
//! Admission is never a lifecycle state; the view only adds a label beside
//! whatever phase the agent is in.

/// A quota group's cooldown as last learned by the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionCooldown {
    /// `until`, `unknown` or `unavailable` (unknown strings are kept as-is).
    pub state: String,
    pub remaining_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionGroupView {
    pub group: String,
    pub cooldown: Option<AdmissionCooldown>,
}

/// Bounded admission view of one agent process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdmissionView {
    pub waiting: u64,
    pub admitted: u64,
    pub longest_wait_seconds: Option<u64>,
    pub groups: Vec<AdmissionGroupView>,
    pub revision: u64,
}

/// Groups are already bounded by the agent's configuration; this guards the
/// TUI against a hostile or buggy peer.
const MAX_GROUPS: usize = 32;
const MAX_GROUP_LABEL: usize = 48;

/// Parse an `admission` object; `None` when absent or not an object.
pub fn parse_admission(
    value: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Option<AdmissionView> {
    let object = value.as_object()?;
    let groups = object
        .get("groups")
        .and_then(|g| g.as_array())
        .map(|groups| {
            groups
                .iter()
                .take(MAX_GROUPS)
                .filter_map(|group| {
                    let name = group.get("group")?.as_str()?;
                    let name: String = sanitize(name).chars().take(MAX_GROUP_LABEL).collect();
                    let cooldown = group.get("cooldown").and_then(|c| {
                        Some(AdmissionCooldown {
                            state: sanitize(c.get("state")?.as_str()?),
                            remaining_seconds: c.get("remainingSeconds").and_then(|r| r.as_u64()),
                        })
                    });
                    Some(AdmissionGroupView {
                        group: name,
                        cooldown,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(AdmissionView {
        waiting: object.get("waiting").and_then(|v| v.as_u64()).unwrap_or(0),
        admitted: object.get("admitted").and_then(|v| v.as_u64()).unwrap_or(0),
        longest_wait_seconds: object.get("longestWaitSeconds").and_then(|v| v.as_u64()),
        groups,
        revision: object.get("revision").and_then(|v| v.as_u64()).unwrap_or(0),
    })
}

impl AdmissionView {
    /// A short status label, or `None` when nothing is worth showing: waiting
    /// attempts first (with the throttled group's cooldown when known), else
    /// an active dated cooldown, else nothing.
    pub fn status_label(&self) -> Option<String> {
        let throttled = self.groups.iter().find(|g| {
            g.cooldown
                .as_ref()
                .is_some_and(|c| c.state != "until" || c.remaining_seconds.unwrap_or(0) > 0)
        });
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
        let throttled = self.groups.iter().find(|g| {
            g.cooldown
                .as_ref()
                .is_some_and(|c| c.state != "until" || c.remaining_seconds.unwrap_or(0) > 0)
        });
        if self.waiting > 0 {
            return Some(match self.longest_wait_seconds {
                Some(seconds) => format!("waiting {seconds}s"),
                None => "waiting".to_string(),
            });
        }
        let cooldown = throttled?.cooldown.as_ref()?;
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
