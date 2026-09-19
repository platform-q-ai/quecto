//! Typed protocol values of the `resume_session` exchange (#2011): what the
//! TUI sends (stable identity, optional explicit action, the home version it
//! was shown) and what the harness answers (restored, cancelled, a typed
//! decision or a typed refusal). Availability is carried, never inferred.
use super::state_payloads::ResumeSessionAck;
use serde::{Deserialize, Serialize};

/// An explicit action of a resume decision, by its stable wire name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeAction {
    OpenOriginal,
    ForkCurrent,
    Locate,
    Associate,
    Cancel,
}

/// The `resume_session` request fields beside the request id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResumeSelection {
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ResumeAction>,
    #[serde(
        rename = "expectedHomeVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub expected_home_version: Option<String>,
}

impl ResumeSelection {
    /// The exact-key restore: `/resume <key>` as typed.
    pub fn exact(session: &str) -> Self {
        Self {
            session: session.to_string(),
            action: None,
            expected_home_version: None,
        }
    }
}

/// Why the session cannot simply be restored in this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeDecisionKind {
    CrossFolder,
    HomeMissing,
    HomeChanged,
    HomeUnknown,
    LegacyUnscoped,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ResumeActionOffer {
    pub action: ResumeAction,
    /// The harness's word; the TUI never upgrades it.
    pub available: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// The typed decision of a `resume_session` answer. Its texts are untrusted
/// metadata: the presentation makes them safe before it renders them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeDecision {
    pub session: String,
    pub session_key: String,
    pub kind: ResumeDecisionKind,
    pub home_version: String,
    pub execution_path: Option<String>,
    pub detail: Option<String>,
    pub actions: Vec<ResumeActionOffer>,
}

/// What a `resume_session` answer says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeAnswer {
    /// History was restored (also every answer of a harness before #2011).
    Resumed(ResumeSessionAck),
    /// The harness acknowledged a cancel: nothing changed.
    Cancelled,
    Decision(ResumeDecision),
    /// A refusal; the stable code when the harness sent one.
    Refused(Option<String>),
}

/// The one typed shape every `resume_session` payload is read through.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct WireAnswer {
    outcome: Option<String>,
    code: Option<String>,
    session: Option<String>,
    session_key: Option<String>,
    kind: Option<ResumeDecisionKind>,
    home_version: Option<String>,
    execution_path: Option<String>,
    detail: Option<String>,
    actions: Vec<ResumeActionOffer>,
}

impl WireAnswer {
    /// A decision only when it is complete and every part of it is known.
    fn into_decision(self) -> Option<ResumeDecision> {
        (!self.actions.is_empty()).then_some(())?;
        Some(ResumeDecision {
            session: self.session?,
            session_key: self.session_key?,
            kind: self.kind?,
            home_version: self.home_version?,
            execution_path: self.execution_path,
            detail: self.detail,
            actions: self.actions,
        })
    }
}

/// Map a `resume_session` response. Only a success that does not say
/// `cancelled` is a restore (a missing `session` falls back to the literal
/// `"session"` so the toast stays user-visible); only a failure that carries
/// a complete, known decision opens a dialog — an unknown kind or action is a
/// plain refusal, never a guessed choice.
pub fn parse_resume_answer(success: bool, data: Option<&serde_json::Value>) -> ResumeAnswer {
    let wire = data
        .and_then(|data| serde_json::from_value::<WireAnswer>(data.clone()).ok())
        .unwrap_or_default();
    match (success, wire.outcome.as_deref()) {
        (true, Some("cancelled")) => ResumeAnswer::Cancelled,
        (true, _) => ResumeAnswer::Resumed(ResumeSessionAck {
            name: wire.session.unwrap_or_else(|| "session".to_string()),
            session_key: wire.session_key,
        }),
        (false, Some("decision")) => wire
            .into_decision()
            .map_or(ResumeAnswer::Refused(None), ResumeAnswer::Decision),
        (false, _) => ResumeAnswer::Refused(wire.code),
    }
}

#[cfg(test)]
#[path = "resume_decision_payloads_tests.rs"]
mod tests;
