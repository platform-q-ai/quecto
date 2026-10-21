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
    /// Broker health (#2024 S3): `connected` | `reconnecting` | `unavailable`.
    /// Carried by `get_state` and by every pushed `admission_state_changed`
    /// of a process bound to an authority (see `uds-protocol.md`); absent for
    /// a process without one, or from an older harness.
    #[serde(default)]
    pub authority_status: Option<String>,
}

/// A `restart_admission` answer (#2615): how the broker came back and the
/// session's link health after rejoining it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionRestartView {
    /// `service_restarted`, `restarted`, `started`, or `unconfirmed`.
    #[serde(default)]
    pub outcome: Option<String>,
    /// With `unconfirmed`: how many other sessions or sub-agents share the
    /// broker.
    #[serde(default)]
    pub shared_with: Option<u64>,
    #[serde(default)]
    pub authority_status: Option<String>,
}

/// Parse a `restart_admission` response's data; anything unreadable is the
/// empty view (sanitized: it is shown to the user).
pub fn parse_admission_restart(
    value: Option<&serde_json::Value>,
    sanitize: &dyn Fn(&str) -> String,
) -> AdmissionRestartView {
    let mut view = value
        .and_then(|value| AdmissionRestartView::deserialize(value).ok())
        .unwrap_or_default();
    view.outcome = view.outcome.map(|outcome| sanitize(&outcome));
    view.authority_status = view.authority_status.map(|status| sanitize(&status));
    view
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

/// The broker-health badge for the footer (#2024 S3, #2615): the health
/// (`admission ✓ connected`, `admission ⟳ reconnecting`, `admission ✗
/// unavailable`) and, while this session has attempts waiting, its own queue
/// depth (`· 2 queued here`). `None` for a status this TUI does not know.
pub fn authority_badge(status: &str, waiting: u64) -> Option<String> {
    let health = match status {
        "connected" => "✓ connected",
        "reconnecting" => "⟳ reconnecting",
        "unavailable" => "✗ unavailable",
        _ => return None,
    };
    Some(match waiting {
        0 => format!("admission {health}"),
        queued => format!("admission {health} · {queued} queued here"),
    })
}

impl AdmissionView {
    fn throttled(&self) -> Option<&AdmissionGroupView> {
        self.groups
            .iter()
            .find(|group| {
                group.cooldown.as_ref().is_some_and(|cooldown| {
                    matches!(cooldown.state.as_str(), "unknown" | "unavailable")
                        || (cooldown.state == "until"
                            && cooldown
                                .remaining_seconds
                                .is_some_and(|seconds| seconds > 0))
                })
            })
            .or_else(|| {
                self.groups.iter().find(|group| {
                    group.cooldown.as_ref().is_some_and(|cooldown| {
                        cooldown.state == "until" && cooldown.remaining_seconds == Some(0)
                    })
                })
            })
    }

    /// Whether a locally projectable dated cooldown still has time remaining.
    pub(crate) fn has_active_dated_cooldown_after(&self, elapsed_seconds: u64) -> bool {
        self.groups.iter().any(|group| {
            group.cooldown.as_ref().is_some_and(|cooldown| {
                cooldown.state == "until"
                    && cooldown
                        .remaining_seconds
                        .is_some_and(|seconds| seconds > elapsed_seconds)
            })
        })
    }

    /// A short status label, or `None` when nothing is worth showing: waiting
    /// attempts first (with the throttled group's cooldown when known), else
    /// an active dated cooldown, else nothing.
    pub fn status_label(&self) -> Option<String> {
        let throttled = self.throttled();
        if self.waiting > 0 {
            let mut label = match self.longest_wait_seconds {
                Some(seconds) => format!("{seconds}s"),
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
    /// survive a narrow sub-agent panel ("4s", "cooldown 30s").
    pub fn compact_label(&self) -> Option<String> {
        if self.waiting > 0 {
            return Some(match self.longest_wait_seconds {
                Some(seconds) => format!("{seconds}s"),
                None => "waiting".to_string(),
            });
        }
        let cooldown = self.throttled()?.cooldown.as_ref()?;
        Some(match cooldown.state.as_str() {
            "until" => match cooldown.remaining_seconds {
                Some(0) => "cooldown elapsed".to_string(),
                Some(seconds) => format!("cooldown {seconds}s"),
                None => "throttled".to_string(),
            },
            "unavailable" => "unavailable".to_string(),
            _ => "throttled".to_string(),
        })
    }

    /// The broker-health badge for the footer (#2024 S3, #2615) from this
    /// view's own `authorityStatus`; `None` when the agent reported none.
    pub fn authority_badge(&self) -> Option<String> {
        authority_badge(self.authority_status.as_deref()?, self.waiting)
    }

    fn cooldown_text(group: &AdmissionGroupView) -> String {
        match group.cooldown.as_ref() {
            Some(cooldown) if cooldown.state == "until" => match cooldown.remaining_seconds {
                Some(0) => format!("{} cooldown elapsed", group.group),
                Some(seconds) => format!("{} cooldown {seconds}s", group.group),
                None => format!("{} throttled", group.group),
            },
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
