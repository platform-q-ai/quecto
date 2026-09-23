//! Bounded inspection wire contract. Both adapters use these affirmative,
//! closed shapes; unknown fields at every object boundary are rejected.
//! Serde's `deny_unknown_fields` closes the affirmative field/type declarations:
//! only declared members are accepted, rather than enumerating forbidden keys.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlStatus {
    Queued,
    Started,
    Completed,
    Failed,
    Cancelled,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReceipt {
    pub id: String,
    pub command: String,
    pub status: ControlStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdmissionSnapshot {
    pub waiting: usize,
    pub admitted: usize,
    /// Longest wait among the sampled waiting attempts; absent when nothing
    /// waits or every waiting attempt is beyond the sample (see `hidden`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub longest_wait_seconds: Option<u64>,
    pub groups: Vec<GroupSnapshot>,
    pub counters: AdmissionCounters,
    /// Waiting or admitted attempts beyond the bounded sample.
    pub hidden: usize,
    /// Advances on every transition (a delta cursor for `admission_state_changed`).
    pub revision: u64,
    /// The authority directory this process is bound to (#2024 S3): on
    /// `get_state` and on this process's own pushed `admission_state_changed`
    /// whenever it is bound to an authority. Absent for a process without an
    /// authority, and never forwarded on a descendant's re-emitted event.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub directory: Option<String>,
    /// The authority epoch this process's capability was minted in (#2024 S3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    /// Whether the authority connection is currently open (#2024 S3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected: Option<bool>,
    /// `connected` | `reconnecting` | `unavailable` (#2024 S3): the health the
    /// TUI footer renders, read from the live link on every `get_state` and
    /// carried on every pushed `admission_state_changed` of a bound process
    /// (including one re-emitted for a descendant). Absent for a process
    /// without an authority.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "authorityStatus"
    )]
    pub authority_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdmissionCounters {
    pub completed: u64,
    pub refused: u64,
    pub cancelled: u64,
    pub abandoned: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupSnapshot {
    pub group: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub cooldown: Option<CooldownSnapshot>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub last_refusal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CooldownSnapshot {
    /// `until`, `unknown` or `unavailable`.
    pub state: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub remaining_seconds: Option<u64>,
}

/// Advisory warning about a usable provider that bypasses broker admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdmissionBindingWarning {
    pub slot: String,
    pub code: String,
    pub message: String,
}

impl AdmissionBindingWarning {
    pub fn new(slot: &str) -> Self {
        Self {
            slot: slot.to_owned(),
            code: "admission_binding_missing".into(),
            message: format!(
                "Provider slot '{slot}' is usable without admission-broker gating; requests may encounter API rate limits. Configure admission.bindings for this slot (with a valid alias and group, or an intentional */default fallback), then restart the broker and agent."
            ),
        }
    }
}

/// Full slim projection. Defaults retain compatibility with older slim senders;
/// present values still have to satisfy the declared wire types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateSnapshot {
    pub state: String,
    #[serde(deserialize_with = "required_nullable_string")]
    pub effort: Option<String>,
    #[serde(default)]
    pub effort_levels: Vec<String>,
    pub model: String,
    #[serde(default)]
    pub session_key: String,
    pub progress: SnapshotProgress,
    pub generation: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub workflow: Option<SnapshotWorkflow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_receipts: Vec<ControlReceipt>,
    #[serde(default)]
    pub automatic_turns_suspended: bool,
    #[serde(default)]
    pub repeated_failure_notifications: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub admission: Option<AdmissionSnapshot>,
    #[serde(default)]
    pub admission_warnings: Vec<AdmissionBindingWarning>,
}

fn required_nullable_string<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

fn present_optional<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotProgress {
    pub state: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotWorkflow {
    pub active_template: SnapshotTemplate,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub current_step: Option<SnapshotStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotTemplate {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotStep {
    pub index: u64,
    pub key: String,
    pub label: String,
    pub phase: String,
    pub done: bool,
}

impl StateSnapshot {
    pub fn is_valid(&self) -> bool {
        self.admission.as_ref().is_none_or(|admission| {
            admission.groups.iter().all(|group| {
                group.cooldown.as_ref().is_none_or(|cooldown| {
                    matches!(cooldown.state.as_str(), "until" | "unknown" | "unavailable")
                })
            })
        })
    }
}
