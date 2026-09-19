//! Typed `search_session_metadata` exchange (#2010): the request the picker's
//! search box sends and the answer it gets. Rows are discovery rows (the same
//! parser as `list_sessions`, so a searched row carries the same home version);
//! the generation is the client's own counter, echoed by the harness, that an
//! answer must carry to be shown.
use super::session_payloads::{ResumeSessionSummary, SessionListScope, parse_resume_sessions};

/// What the search box asks: literal text, the scope on screen, and the
/// generation of this edit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SessionSearchRequest {
    pub query: String,
    pub scope: SessionListScope,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearchAnswer {
    /// The generation the harness echoed; `None` when it sent none.
    pub generation: Option<u64>,
    pub sessions: Vec<ResumeSessionSummary>,
    /// Matches in scope, of which `sessions` may be a prefix.
    pub total_matches: u64,
    /// Why the harness searched nothing, when it refused the query.
    pub refused: Option<String>,
    /// The harness cut the rows at its limit: `total_matches` is the whole.
    pub truncated: bool,
    /// The scope the harness searched, echoed; `None` when it sent none.
    pub scope: Option<SessionListScope>,
}

/// The answer's own fields, typed; rows are mapped by the discovery-row parser.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchEnvelope {
    generation: Option<u64>,
    total_matches: Option<u64>,
    refused: Option<String>,
    truncated: Option<bool>,
}

/// An envelope that does not decode (a generation that is no number) yields
/// no generation, so the answer can never be taken for the latest.
pub fn parse_session_search(data: &serde_json::Value) -> SessionSearchAnswer {
    use serde::Deserialize;
    let envelope = SearchEnvelope::deserialize(data).unwrap_or_default();
    let sessions = parse_resume_sessions(data);
    let total_matches = envelope.total_matches.unwrap_or(sessions.len() as u64);
    // Decoded on its own: an unknown scope is no scope, not a lost envelope.
    let scope = data.get("scope").cloned();
    SessionSearchAnswer {
        generation: envelope.generation,
        total_matches,
        refused: envelope
            .refused
            .map(|reason| reason.chars().take(200).collect()),
        truncated: envelope.truncated.unwrap_or(false) || total_matches > sessions.len() as u64,
        scope: scope.and_then(|scope| serde_json::from_value(scope).ok()),
        sessions,
    }
}

#[cfg(test)]
#[path = "session_search_payloads_tests.rs"]
mod tests;
