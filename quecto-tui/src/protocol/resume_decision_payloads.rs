//! Typed values for `resume_session`. A resume either succeeds or is refused;
//! the TUI never offers a follow-up action.
use super::state_payloads::ResumeSessionAck;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResumeSelection {
    pub session: String,
    #[serde(
        rename = "expectedHomeVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub expected_home_version: Option<String>,
}

impl ResumeSelection {
    pub fn exact(session: &str) -> Self {
        Self {
            session: session.to_string(),
            expected_home_version: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeRefusalCode {
    BelongsElsewhere,
    HomeMissing,
    HomeChanged,
    HomeUnknown,
    NoHomeRecorded,
}

/// Untrusted refusal metadata. All strings are sanitized at the presentation boundary.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeRefusal {
    pub code: ResumeRefusalCode,
    pub kind: String,
    pub execution_path: Option<String>,
    pub detail: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeAnswer {
    Resumed(ResumeSessionAck),
    Refused(Option<ResumeRefusal>),
    Unrecognized(Option<String>),
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct WireAnswer {
    outcome: Option<String>,
    session: Option<String>,
    session_key: Option<String>,
    code: Option<ResumeRefusalCode>,
    kind: Option<String>,
    execution_path: Option<String>,
    detail: Option<String>,
    command: Option<String>,
}

impl WireAnswer {
    fn into_refusal(self) -> Option<ResumeRefusal> {
        Some(ResumeRefusal {
            code: self.code?,
            kind: self.kind?,
            execution_path: self.execution_path,
            detail: self.detail,
            command: self.command,
        })
    }
}

/// Interpret only explicitly understood outcomes. Unknown or malformed values never
/// mutate local session state.
pub fn parse_resume_answer(success: bool, data: Option<&serde_json::Value>) -> ResumeAnswer {
    let wire = match data.map(|v| serde_json::from_value::<WireAnswer>(v.clone())) {
        None => WireAnswer::default(),
        Some(Ok(wire)) => wire,
        Some(Err(_)) if success => return ResumeAnswer::Unrecognized(None),
        Some(Err(_)) => return ResumeAnswer::Refused(None),
    };
    match (success, wire.outcome.as_deref()) {
        (true, Some("resumed") | None) => ResumeAnswer::Resumed(ResumeSessionAck {
            name: wire.session.unwrap_or_else(|| "session".to_string()),
            session_key: wire.session_key,
        }),
        (false, Some("refused")) => ResumeAnswer::Refused(wire.into_refusal()),
        (false, _) => ResumeAnswer::Refused(None),
        (true, Some(_)) => ResumeAnswer::Unrecognized(wire.outcome),
    }
}
