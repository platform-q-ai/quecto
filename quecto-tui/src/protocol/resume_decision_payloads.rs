//! Typed protocol values of the `resume_session` exchange (#2011): what the
//! TUI sends (stable identity, optional explicit action, the home version it
//! was shown) and what the harness answers (restored, cancelled, a typed
//! decision or a typed refusal). Availability is carried, never inferred.
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

/// The typed decision of a `resume_session` answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeDecision {
    pub session: String,
    pub session_key: String,
    pub kind: ResumeDecisionKind,
    pub home_version: String,
    #[serde(default)]
    pub execution_path: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    pub actions: Vec<ResumeActionOffer>,
}

/// What a `resume_session` answer says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeAnswer {
    /// History was restored (also every answer of a harness before #2011).
    Resumed,
    /// The harness acknowledged a cancel: nothing changed.
    Cancelled,
    Decision(ResumeDecision),
    /// A refusal; the stable code when the harness sent one.
    Refused(Option<String>),
}

/// Map a `resume_session` response. Only a success that does not say
/// `cancelled` is a restore; only a failure that carries a complete, known
/// decision opens a dialog — an unknown kind or action is a plain refusal,
/// never a guessed choice. Every text is made safe for the terminal.
pub fn parse_resume_answer(success: bool, data: Option<&serde_json::Value>) -> ResumeAnswer {
    let outcome = data
        .and_then(|data| data.get("outcome"))
        .and_then(serde_json::Value::as_str);
    match (success, outcome) {
        (true, Some("cancelled")) => ResumeAnswer::Cancelled,
        (true, _) => ResumeAnswer::Resumed,
        (false, Some("decision")) => data
            .and_then(|data| serde_json::from_value::<ResumeDecision>(data.clone()).ok())
            .filter(|decision| !decision.actions.is_empty())
            .map_or(ResumeAnswer::Refused(None), |decision| {
                ResumeAnswer::Decision(safe_decision(decision))
            }),
        (false, _) => ResumeAnswer::Refused(
            data.and_then(|data| data.get("code"))
                .and_then(serde_json::Value::as_str)
                .map(safe_text),
        ),
    }
}

fn safe_decision(mut decision: ResumeDecision) -> ResumeDecision {
    decision.session = safe_text(&decision.session);
    decision.execution_path = decision.execution_path.as_deref().map(safe_text);
    decision.detail = decision.detail.as_deref().map(safe_text);
    for offer in &mut decision.actions {
        offer.reason = offer.reason.as_deref().map(safe_text);
    }
    decision
}

/// Untrusted metadata: no terminal controls, bounded length.
fn safe_text(value: &str) -> String {
    crate::components::ansi::sanitize_control(value)
        .chars()
        .take(512)
        .collect()
}

#[cfg(test)]
#[path = "resume_decision_payloads_tests.rs"]
mod tests;
