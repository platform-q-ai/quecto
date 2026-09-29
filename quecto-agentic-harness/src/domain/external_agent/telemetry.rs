//! What a claude-code member's event log records (#2304): its turns, its
//! tool calls, its lifecycle and what its stream said that the vocabulary
//! could not read. Ids, kinds, sizes and durations only: never a prompt, an
//! assistant's text or thinking, a tool's arguments or output, or a
//! credential. Text from the stream (a name, an id, a reason) is kept only
//! bounded to an allowlist and when it does not look like a secret
//! ([`recorded_text`]); an unknown event's type only as a [`fingerprint`].
//!
//! Each record is filed as an `external_agent_*` event of
//! [`crate::domain::audit::AuditEvent`] beside the `member_ref` it belongs
//! to; the envelope carries `session`, `parent` and `turn`, so a record's
//! own turn is its `member_turn` (the envelope's is 32 bits wide).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::stream::{AssistantContent, ExternalAgentEvent};
use super::usage::CostDrop;
use crate::domain::redaction::redact_secrets;

/// The most of a name or an id from the stream a record keeps, in bytes.
pub const RECORDED_NAME_BYTES: usize = 64;

/// One member turn, as it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentTurn {
    /// The member's turn (the envelope's `turn` is it, saturated).
    pub member_turn: u64,
    /// `completed`, `failed`, `aborted`, `budget_exceeded`, `exited` (its
    /// process's output ended first), `closed` or `abandoned` (the member
    /// was closed, or ended as its state became unknown, while it ran).
    pub turn_end: String,
    /// Why a turn that did not complete ended: its `terminal_reason`, an
    /// `api_<status>`, the assistant's error kind, or a kind taken from its
    /// first error ([`error_reason_kind`]).
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
    /// The process's cumulative cost went down during this turn (the CLI
    /// reports 0 after some errors): nothing was charged for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_drop: Option<CostDrop>,
}

/// One tool call, once answered (or once its turn ended without an
/// answer): its name, id and sizes, never its arguments or output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentTool {
    #[serde(default)]
    pub member_turn: Option<u64>,
    /// The tool's name ([`recorded_id`]).
    pub tool: String,
    /// The call's id ([`recorded_id`]): for display only; results are
    /// paired with calls on the id as the stream gave it.
    pub tool_use_id: String,
    /// From the call to its result, on the session's clock.
    pub duration_ms: u64,
    /// `ok`, `error`, `denied` (a permission rule refused it) or
    /// `unanswered` (its turn ended first).
    pub outcome: String,
    /// The denylist rule that refused it, once a hook names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    /// The call's input, as JSON, in bytes.
    pub argument_bytes: usize,
    /// Its result's text, in bytes.
    pub result_bytes: usize,
    /// The board task a board tool's call names ([`board_task_id`]).
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
    /// A prompt was refused: its refusal's kind (`busy`, `ended` …), never
    /// the prompt.
    PromptRefused { refusal: String },
    /// A dequeued follow-up could not be written: its refusal's kind,
    /// never the text.
    FollowUpFailed { member_turn: u64, refusal: String },
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
    /// The event log itself lost records: `dropped` never reached its
    /// writer (its queue was full), `failed` could not be written. The
    /// log's last record, written only when it lost any.
    LogIncomplete { dropped: u64, failed: u64 },
}

/// Something the stream said that the vocabulary could not read: counted,
/// and recorded at the 1st, 2nd, 4th, 8th … occurrence of its kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAgentStreamDiagnostic {
    /// `skipped_line` or `unknown_event`.
    pub kind: String,
    /// A skipped line's reason (`over_cap`, `not_utf8`), or an unknown
    /// event's type's [`fingerprint`] (never the type: any text may stand
    /// there).
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
    use StreamTelemetry::{Ignored, Recorded};
    match event {
        ExternalAgentEvent::Init(_) => Recorded("external_agent_lifecycle"),
        ExternalAgentEvent::AssistantBlock { block, .. } => match block {
            AssistantContent::ToolUse { .. } => Recorded("external_agent_tool"),
            // An assistant's text and thinking are never logged.
            AssistantContent::Text(_) | AssistantContent::Thinking { .. } => Ignored,
        },
        ExternalAgentEvent::AssistantError { .. } | ExternalAgentEvent::Result(_) => {
            Recorded("external_agent_turn")
        }
        ExternalAgentEvent::ToolResult(_) => Recorded("external_agent_tool"),
        ExternalAgentEvent::Unknown { .. } | ExternalAgentEvent::LineSkipped(_) => {
            Recorded("external_agent_stream_diagnostic")
        }
        // Rate limits feed the admission record (#2290); background tasks
        // and user text carry nothing the log keeps; an interrupt's answer
        // is recorded when it is written.
        ExternalAgentEvent::ThinkingTokens { .. }
        | ExternalAgentEvent::UserText { .. }
        | ExternalAgentEvent::TaskStarted(_)
        | ExternalAgentEvent::TaskNotification(_)
        | ExternalAgentEvent::BackgroundTasksChanged { .. }
        | ExternalAgentEvent::RateLimit(_)
        | ExternalAgentEvent::InterruptAnswered(_) => Ignored,
    }
}

/// Whether `name` is kept by a record as it is: 1 to
/// [`RECORDED_NAME_BYTES`] bytes of ASCII letters, digits and `_ - . :`.
pub fn is_recorded_name(name: &str) -> bool {
    (1..=RECORDED_NAME_BYTES).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
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

/// How much of a text [`recorded_text`] judges as it came: every byte a
/// [`recorded_name`] can keep (each of its characters is at most four
/// bytes), so a label before them is seen.
const JUDGED_TEXT_BYTES: usize = 4 * RECORDED_NAME_BYTES;

/// Text from the stream as a record keeps it (#2304 swarm review): its
/// [`recorded_name`], only when neither the text as it came (its first
/// [`JUDGED_TEXT_BYTES`]) nor the name looks like a secret to the shared
/// redaction ([`redact_secrets`]); else `None`. Bounding and filtering
/// alone are no redaction: `sk-ant-…` is all allowlisted characters.
pub fn recorded_text(text: &str) -> Option<String> {
    let recorded = recorded_name(text);
    let judged = char_prefix(text, JUDGED_TEXT_BYTES);
    let clean = |text: &str| redact_secrets(text) == text;
    match (clean(judged), clean(&recorded)) {
        (true, true) => Some(recorded),
        _ => None,
    }
}

/// An id or a name a record must hold: its [`recorded_text`], else its
/// [`fingerprint`].
pub fn recorded_id(text: &str) -> String {
    let id = recorded_text(text).unwrap_or_else(|| fingerprint(text));
    assert!(id.len() <= RECORDED_NAME_BYTES && id.is_ascii());
    id
}

/// The hex digits of a [`fingerprint`]'s digest kept.
const FINGERPRINT_HEX_DIGITS: usize = 16;

/// A fixed-size stand-in for text a record must never keep: `sha256:` and
/// the first [`FINGERPRINT_HEX_DIGITS`] hex digits of the text's SHA-256
/// digest. The same text has the same print, so kinds are still told
/// apart and counted.
pub fn fingerprint(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0xf])
        .take(FINGERPRINT_HEX_DIGITS)
        .map(|nibble| char::from_digit(u32::from(nibble), 16).expect("a nibble is a hex digit"))
        .collect();
    let print = format!("sha256:{hex}");
    assert_eq!(print.len(), "sha256:".len() + FINGERPRINT_HEX_DIGITS);
    print
}

/// A tool call's id as its call and result are paired on it: the SHA-256
/// digest of the whole id, a fixed 32 bytes however long the id.
pub type CallKey = [u8; 32];

/// The [`CallKey`] of tool call id `id`.
pub fn call_key(id: &str) -> CallKey {
    Sha256::digest(id.as_bytes()).into()
}

/// At most `bytes` of `text`, cut on a character boundary.
fn char_prefix(text: &str, bytes: usize) -> &str {
    let mut cut = bytes.min(text.len());
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    &text[..cut]
}

/// The most of a reason taken from a result's `errors[]` a record keeps,
/// in bytes.
pub const ERROR_REASON_BYTES: usize = 32;

/// The most words of an error a reason kind is taken from.
const ERROR_REASON_WORDS: usize = 3;

/// The longest word an error's reason kind is taken from: a longer run of
/// letters and digits is an id or a key, not a word.
const ERROR_WORD_BYTES: usize = 24;

/// A turn's reason kind from its result's first error, for a turn whose
/// result names no `terminal_reason`: the error is redacted, then its
/// leading words (runs of ASCII letters and digits, lowercased) are joined
/// by `_`, up to and including the first all-digit word (a status), at
/// most [`ERROR_REASON_WORDS`] of them, stopping before a word longer than
/// [`ERROR_WORD_BYTES`], and cut to [`ERROR_REASON_BYTES`]. So `API Error:
/// 529 Overloaded` is `api_error_529`. `None` when no word leads it.
pub fn error_reason_kind(errors: &[String]) -> Option<String> {
    let redacted = redact_secrets(errors.first()?);
    let mut words: Vec<String> = Vec::new();
    for word in redacted.split(|c: char| !c.is_ascii_alphanumeric()) {
        match word.len() {
            0 => continue,
            1..=ERROR_WORD_BYTES => words.push(word.to_ascii_lowercase()),
            _ => break,
        }
        let status = word.bytes().all(|byte| byte.is_ascii_digit());
        if status || words.len() == ERROR_REASON_WORDS {
            break;
        }
    }
    let mut kind = words.join("_");
    kind.truncate(ERROR_REASON_BYTES);
    assert!(kind.len() <= ERROR_REASON_BYTES && kind.is_ascii());
    match kind.len() {
        0 => None,
        // Words that together look like a secret are not kept either.
        _ => recorded_text(&kind),
    }
}

/// The MCP servers a member's board tools come from: quecto's, and the
/// spike's `board` (#2264). `mcp__board__` is the spike's prefix only: drop
/// it once #2289 gives members quecto's own board server.
const BOARD_TOOL_PREFIXES: &[&str] = &["mcp__quecto__", "mcp__board__"];

/// The board task a board tool's call names (its `task_id`): a number, or
/// text [`recorded_text`] keeps; `None` for any other tool, or an id that
/// looks like a secret. Nothing else of a call's input is kept.
pub fn board_task_id(tool: &str, input: &serde_json::Value) -> Option<String> {
    let board = BOARD_TOOL_PREFIXES
        .iter()
        .any(|prefix| tool.starts_with(prefix));
    match (board, input.get("task_id")) {
        (true, Some(serde_json::Value::String(id))) => recorded_text(id),
        (true, Some(serde_json::Value::Number(id))) => recorded_text(&id.to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
pub(crate) mod tests;
