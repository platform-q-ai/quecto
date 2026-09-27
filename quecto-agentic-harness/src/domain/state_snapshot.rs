//! Bounded inspection wire contract. These affirmative, closed shapes are
//! what this harness writes: `deny_unknown_fields` closes each declaration,
//! so only declared members deserialize, with their declared types.
//!
//! A reader of another process's projection (a parent reading a child's
//! busy snapshot) goes through [`StateSnapshot::read_forward_compatible`]
//! and [`UnchangedSnapshot::read`] instead (#2210 review): at every object
//! boundary the members this harness knows are read by these closed types,
//! and a member it does not know, when its name has a member's shape, is an
//! additive member of a newer writer — dropped, never refused, never passed
//! on. See `state_snapshot_reader.rs`.
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
    /// The model request in flight (#2210): present only while the agent
    /// waits on the model — thinking or streaming — and absent otherwise.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub model_turn: Option<ModelTurnSnapshot>,
}

/// The members of the projection that are live measurements rather than
/// transitions: `generation` does not track them, so a `since` poll whose
/// cursor is current still carries each one present (#2210 review). The
/// one rule, used by the child that answers and the parent that relays.
pub const SINCE_BYPASSING_MEMBERS: &[&str] = &["modelTurn"];

/// The answer to a `since` poll whose cursor is current: the unchanged
/// marker, carrying the live measurements [`SINCE_BYPASSING_MEMBERS`] names
/// when present (#2210 review) — a model turn stays visible, and a poll
/// stays small.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnchangedSnapshot {
    /// Always `true`.
    pub unchanged: bool,
    pub generation: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub model_turn: Option<ModelTurnSnapshot>,
}

impl UnchangedSnapshot {
    /// The unchanged marker at `generation` for a projection `snapshot`,
    /// with its live measurements.
    pub fn at(generation: u64, snapshot: &StateSnapshot) -> Self {
        Self {
            unchanged: true,
            generation,
            model_turn: snapshot.model_turn.clone(),
        }
    }

    /// The same marker at another `generation` (a caller's cursor).
    pub fn with_generation(self, generation: u64) -> Self {
        Self { generation, ..self }
    }
}

/// The model request in flight (#2210): how long it has run and what its
/// attempt in flight has streamed, so a supervisor can tell a live but
/// runaway reply (output keeps growing) from a hung one (no events).
/// Live measurements: they are not folded into `generation`, so a `since`
/// cursor that reads `unchanged` says nothing about them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelTurnSnapshot {
    /// Since the request started, retries and admission waits included.
    pub elapsed_ms: u64,
    /// Each attempt's output cap in bytes, when one applies.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub output_cap_bytes: Option<u64>,
    /// The attempt in flight; absent before one starts (an admission wait,
    /// a retry's back-off) and on a path that observes no attempts.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub attempt: Option<AttemptProgressSnapshot>,
}

/// The attempt in flight of a model request (#2210).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptProgressSnapshot {
    /// The attempt's number within its request, from 1.
    pub number: u32,
    /// Since the attempt started.
    pub elapsed_ms: u64,
    /// Provider events (SSE `data:` lines) it has received.
    pub events: u32,
    /// Bytes of output it has streamed (text, thinking, refusal and tool-call
    /// argument deltas).
    pub output_bytes: u64,
    /// Since its last event; absent before its first.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub since_last_event_ms: Option<u64>,
    /// When its first token arrived, from its start; absent before one did.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_optional"
    )]
    pub first_token_ms: Option<u64>,
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

#[path = "state_snapshot_reader.rs"]
mod reader;

#[cfg(test)]
#[path = "state_snapshot_tests.rs"]
mod tests;
