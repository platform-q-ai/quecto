//! What a claude-code member's event log records (#2304): its turns, its
//! tool calls, its lifecycle and what its stream said that the vocabulary
//! could not read. Ids, kinds, sizes and durations only: never a prompt, an
//! assistant's text or thinking, or a credential; a tool's summary is
//! redacted ([`crate::domain::redaction`]) and bounded.
//!
//! Each record is filed as an `external_agent_*` event of
//! [`crate::domain::audit::AuditEvent`] beside the `member_ref` it belongs
//! to; the envelope carries `session`, `parent` and `turn`, so a record's
//! own turn is its `member_turn` (the envelope's is 32 bits wide).
#![allow(dead_code, unused_imports)] // red-phase stub (#2304)

use serde::{Deserialize, Serialize};

use super::stream::{AssistantContent, ExternalAgentEvent};
use crate::domain::redaction::{Redacted, redact_url_userinfo};

/// The most of a tool call's summary a record keeps, in bytes.
pub const TOOL_SUMMARY_BYTES: usize = 256;

/// The most of a name or an id from the stream a record keeps, in bytes.
pub const RECORDED_NAME_BYTES: usize = 64;

/// One member turn, as it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentTurn {
    /// The member's turn (the envelope's `turn` is it, saturated).
    pub member_turn: u64,
    /// `completed`, `failed`, `aborted`, `budget_exceeded` or `exited`.
    pub turn_end: String,
    /// Why a turn that did not complete ended: its `terminal_reason`, an
    /// `api_<status>`, or the assistant's error kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_kind: Option<String>,
    #[serde(default)]
    pub claude_session_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub num_turns: Option<u32>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub duration_api_ms: Option<u64>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// This turn's charge at list price: the growth of the process's
    /// cumulative `total_cost_usd`, micro-USD.
    #[serde(default)]
    pub list_price_cost_micro_usd: u64,
    /// The session's charge at list price so far, micro-USD.
    #[serde(default)]
    pub list_price_total_micro_usd: u64,
    /// The board task the member held when the turn started.
    #[serde(default)]
    pub task_id: Option<String>,
}

/// One tool call, once answered (or once its turn ended without an answer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentTool {
    #[serde(default)]
    pub member_turn: Option<u64>,
    pub tool: String,
    pub tool_use_id: String,
    /// From the call to its result, on the session's clock.
    pub duration_ms: u64,
    /// `ok`, `error`, `denied` (a permission rule refused it) or
    /// `unanswered` (its turn ended first).
    pub outcome: String,
    /// The denylist rule that refused it, once a hook names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    pub argument_bytes: usize,
    pub result_bytes: usize,
    /// For Bash the command, for a file tool the path, for a board tool its
    /// ids: redacted and at most [`TOOL_SUMMARY_BYTES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Redacted>,
    #[serde(default)]
    pub task_id: Option<String>,
}

/// A step of the member's life.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExternalAgentLifecycle {
    /// The agent process started under this credential mode (never the
    /// credential).
    Started { credential_mode: String },
    /// It could not be started: the launch error's kind.
    StartRefused { reason: String },
    /// The agent's first `system/init` of a process.
    Initialized {
        cli_version: Option<String>,
        claude_session_id: Option<String>,
        model: Option<String>,
    },
    /// A running turn was interrupted: `abort`, or `lost`.
    Interrupted { member_turn: u64, cause: String },
    /// quecto's abort: follow-ups dropped, a running turn interrupted.
    Aborted {
        member_turn: Option<u64>,
        dropped_follow_ups: usize,
    },
    /// The member was ended: its state became unknown.
    Abandoned {
        member_turn: u64,
        dropped_follow_ups: usize,
    },
    /// The member was closed.
    Closed {
        member_turn: Option<u64>,
        dropped_follow_ups: usize,
    },
    /// The agent's output ended and its process exited.
    Ended {
        clean: bool,
        exit_code: Option<i32>,
        signal: Option<i32>,
        /// From the start, on the session's clock.
        wall_ms: Option<u64>,
    },
}

/// Something the stream said that the vocabulary could not read: counted,
/// and recorded at the 1st, 2nd, 4th, 8th … occurrence of its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentStreamDiagnostic {
    /// `skipped_line` or `unknown_event`.
    pub kind: String,
    /// A skipped line's reason (`over_cap`, `not_utf8`), or an unknown
    /// event's type.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    /// How many of this kind and name the session has seen.
    pub count: u64,
    #[serde(default)]
    pub member_turn: Option<u64>,
}

/// What the event log does with one event of the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamTelemetry {
    /// It feeds the named `external_agent_*` event.
    Recorded(&'static str),
    /// It is deliberately not logged (content, or a later slice's).
    Ignored,
}

/// Every event of the vocabulary, mapped: a new variant does not compile
/// until it is placed here.
pub fn stream_telemetry(event: &ExternalAgentEvent) -> StreamTelemetry {
    let _ = event;
    StreamTelemetry::Ignored
}

/// A name or an id from the stream as a record keeps it: at most
/// [`RECORDED_NAME_BYTES`], ASCII letters, digits and `_ - . :` only, every
/// other character becoming `?`.
pub fn recorded_name(name: &str) -> String {
    let recorded: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' | '.' | ':' => c,
            _ => '?',
        })
        .take(RECORDED_NAME_BYTES)
        .collect();
    assert!(recorded.len() <= RECORDED_NAME_BYTES);
    recorded
}

/// The MCP servers a member's board tools come from: quecto's, and the
/// spike's `board` (#2264).
const BOARD_TOOL_PREFIXES: &[&str] = &["mcp__quecto__", "mcp__board__"];

/// The input fields a tool's summary is taken from, by tool: the command,
/// the path, or a board tool's ids. Any other tool has none.
fn summary_fields(tool: &str) -> &'static [&'static str] {
    match tool {
        "Bash" => &["command"],
        "Edit" | "MultiEdit" | "Write" | "Read" | "NotebookEdit" => &["file_path", "notebook_path"],
        board
            if BOARD_TOOL_PREFIXES
                .iter()
                .any(|prefix| board.starts_with(prefix)) =>
        {
            &["task_id", "member", "message_id"]
        }
        _ => &[],
    }
}

/// The board task a board tool's call names (its `task_id`), bounded as
/// [`recorded_name`] bounds it; `None` for any other tool.
pub fn board_task_id(tool: &str, input: &serde_json::Value) -> Option<String> {
    let _ = (tool, input);
    None
}

/// A tool call's summary: its [`summary_fields`], redacted, then cut to
/// [`TOOL_SUMMARY_BYTES`] on a character boundary; `None` when it has none.
pub fn tool_summary(tool: &str, input: &serde_json::Value) -> Option<Redacted> {
    let _ = (tool, input);
    None
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
