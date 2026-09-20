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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeRefusalCode {
    BelongsElsewhere,
    HomeMissing,
    HomeChanged,
    HomeUnknown,
    NoHomeRecorded,
}

impl ResumeRefusalCode {
    /// The refusals that are about where the session lives — an affirmative
    /// list: any other code is a plain refusal, never a panel.
    fn of(wire: &str) -> Option<Self> {
        Some(match wire {
            "belongs_elsewhere" => Self::BelongsElsewhere,
            "home_missing" => Self::HomeMissing,
            "home_changed" => Self::HomeChanged,
            "home_unknown" => Self::HomeUnknown,
            "no_home_recorded" => Self::NoHomeRecorded,
            _ => return None,
        })
    }

    /// A pre-#2045 harness answered `outcome: "decision"` and named only the
    /// kind: the same five situations, under their older names.
    fn of_kind(kind: &str) -> Option<Self> {
        Some(match kind {
            "cross_folder" => Self::BelongsElsewhere,
            "home_missing" => Self::HomeMissing,
            "home_changed" => Self::HomeChanged,
            "home_unknown" => Self::HomeUnknown,
            "legacy_unscoped" => Self::NoHomeRecorded,
            _ => return None,
        })
    }
}

/// Untrusted refusal metadata. All strings are sanitized at the presentation boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeRefusal {
    pub code: ResumeRefusalCode,
    /// The session that was asked for, to name it as the picker did.
    pub session_key: Option<String>,
    pub kind: String,
    pub execution_path: Option<String>,
    pub detail: Option<String>,
    /// Shell command that opens quecto in the session's folder, when the
    /// harness could spell one that reads the way it runs.
    pub command: Option<String>,
    /// What to type there (`/resume <key>`): `quecto-tui` takes no session flag.
    pub resume: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeAnswer {
    Resumed(ResumeSessionAck),
    /// The session lives elsewhere (or nowhere known): the plain notice.
    Elsewhere(ResumeRefusal),
    /// Any other refusal, with its code when one was readable: a toast.
    Refused(Option<String>),
    Unrecognized(Option<String>),
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct WireAnswer {
    outcome: Option<String>,
    session: Option<String>,
    session_key: Option<String>,
    code: Option<String>,
    kind: Option<String>,
    execution_path: Option<String>,
    detail: Option<String>,
    command: Option<String>,
    resume: Option<String>,
}

impl WireAnswer {
    /// A refusal this TUI can show as the notice, or the plain code.
    fn into_refusal(self, by_kind: bool) -> ResumeAnswer {
        let code = if by_kind {
            self.kind.as_deref().and_then(ResumeRefusalCode::of_kind)
        } else {
            self.code.as_deref().and_then(ResumeRefusalCode::of)
        };
        match (code, self.kind) {
            (Some(code), Some(kind)) => ResumeAnswer::Elsewhere(ResumeRefusal {
                code,
                session_key: self.session_key,
                kind,
                execution_path: self.execution_path,
                detail: self.detail,
                command: self.command,
                resume: self.resume,
            }),
            _ => ResumeAnswer::Refused(self.code),
        }
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
        (false, Some("refused")) => wire.into_refusal(false),
        // An older harness's decision: the same notice, nothing to run.
        (false, Some("decision")) => wire.into_refusal(true),
        (false, _) => ResumeAnswer::Refused(None),
        (true, Some(_)) => ResumeAnswer::Unrecognized(wire.outcome),
    }
}

#[cfg(test)]
#[path = "resume_decision_payloads_tests.rs"]
mod tests;
