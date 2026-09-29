//! The typed vocabulary of an external agent's event stream (#2285).
//!
//! These are the events a `claude -p --output-format stream-json` process
//! emits, typed from the real captures of spike #2264. The infrastructure
//! codec decodes one NDJSON line into these values; the application
//! projection folds them. Fields another CLI version may omit are
//! `Option`s. Only a tool's own `input` and result `content` stay opaque
//! JSON: they are whatever the tool takes and returns.

use serde_json::Value;

/// One event of an external agent's stream.
#[derive(Debug, Clone, PartialEq)]
pub enum ExternalAgentEvent {
    /// `system/init`: the session's configuration. Re-emitted every turn.
    Init(InitEvent),
    /// `system/thinking_tokens`: the model is thinking.
    ThinkingTokens { estimated_tokens: Option<u64> },
    /// One content block of an assistant API message. One API message
    /// arrives as several of these, sharing `message_id`.
    AssistantBlock {
        message_id: String,
        block: AssistantContent,
    },
    /// An assistant event marked with an `error` (e.g.
    /// `authentication_failed`): the turn's model call failed.
    AssistantError {
        message_id: Option<String>,
        kind: String,
    },
    /// A `tool_result` block of a `user` event.
    ToolResult(ToolResultEvent),
    /// A user text block, echoed only with `--replay-user-messages`.
    UserText { text: String },
    /// `system/task_started`: a Bash command became a tracked task.
    TaskStarted(TaskStarted),
    /// `system/task_notification`: a tracked task changed status.
    TaskNotification(TaskNotification),
    /// `system/background_tasks_changed`: the current background tasks.
    BackgroundTasksChanged { tasks: Vec<BackgroundTask> },
    /// `rate_limit_event`: the account's rate-limit standing.
    RateLimit(RateLimitInfo),
    /// `result`: the end of one user turn.
    Result(ResultEvent),
    /// `control_response`: the answer to an interrupt, the only control
    /// request a member sends (#2287). The stopped turn's own `result`, if
    /// a turn was running, may come before or after it.
    InterruptAnswered(InterruptReceipt),
    /// An event type (or `system` subtype, or assistant block type) this
    /// vocabulary does not know. Logged by the codec, never a panic.
    Unknown { kind: String },
    /// A line of the stream that could not be read and was skipped
    /// (#2286). It may have been the turn's `result`, so the session can
    /// end the turn on it instead of waiting for a `result` that never
    /// comes.
    LineSkipped(SkippedLine),
}

/// A skipped line of the stream: why, and how long it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedLine {
    pub reason: SkippedLineReason,
    /// The line's length on the wire, in bytes (its newline included).
    pub bytes: usize,
}

/// Why a line of the stream was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkippedLineReason {
    /// Longer than the adapter reads as one line.
    OverCap,
    /// Not UTF-8, so not JSON.
    NotUtf8,
}

/// What an interrupt's `control_response` says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterruptReceipt {
    /// Whether the agent accepted the interrupt (`subtype: success`).
    pub accepted: bool,
    /// The ids of queued user turns the interrupt withdrew
    /// (`cancel_queued`): no `result` will ever name them.
    pub cancelled: Vec<String>,
}

/// `system/init`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitEvent {
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub tools: Vec<String>,
    pub mcp_servers: Vec<McpServerStatus>,
    pub api_key_source: Option<String>,
    pub permission_mode: Option<String>,
    /// `claude_code_version`: the CLI's version, e.g. `2.1.280`.
    pub cli_version: Option<String>,
    /// `capabilities`: the protocol capabilities the CLI advertises (an
    /// open set; absent on older CLIs).
    pub capabilities: Vec<String>,
}

/// The first Claude Code version known to name, in every result of a turn
/// it ran, the user turns it consumed (`user_message_uuids`): 2.1.280, the
/// version spike #2264 and #2287 were verified against. The CLI advertises
/// no capability for it, so the version is the only word before a result.
pub const NAMES_TURNS_SINCE: [u64; 3] = [2, 1, 280];

/// The capability of a CLI whose interrupt honours `cancel_queued`: the
/// user turns queued behind the running turn are withdrawn, and named in
/// the interrupt's answer, rather than run afterwards.
pub const CANCEL_QUEUED_CAPABILITY: &str = "interrupt_cancel_queued_v1";

/// A plain release's `major.minor.patch`: exactly three dot-separated
/// runs of ASCII digits. Anything else is `None`, which fails safe (not
/// known to name turns): a pre-release such as `2.1.280-beta.1` precedes
/// its release, so it is not the version verified, and a build or other
/// suffix is not a version this reads.
fn release(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split('.').map(|part| {
        match !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
            true => part.parse::<u64>().ok(),
            false => None,
        }
    });
    let release = [parts.next()??, parts.next()??, parts.next()??];
    match parts.next() {
        None => Some(release),
        Some(_) => None,
    }
}

/// One MCP server's connection status in `system/init`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerStatus {
    pub name: String,
    pub status: String,
}

/// The status an MCP server reports once it is usable.
pub const MCP_SERVER_CONNECTED: &str = "connected";

impl InitEvent {
    /// Whether this CLI names the user turns its results consumed: its
    /// version is at least [`NAMES_TURNS_SINCE`]. An absent or unreadable
    /// version is not known to.
    pub fn names_turns(&self) -> bool {
        self.cli_version
            .as_deref()
            .and_then(release)
            .is_some_and(|version| version >= NAMES_TURNS_SINCE)
    }

    /// Whether this CLI's interrupt withdraws the queued user turns: it
    /// advertises [`CANCEL_QUEUED_CAPABILITY`].
    pub fn cancels_queued(&self) -> bool {
        self.capabilities
            .iter()
            .any(|capability| capability == CANCEL_QUEUED_CAPABILITY)
    }

    /// Whether every MCP server reports [`MCP_SERVER_CONNECTED`]: the
    /// launch check a member must pass before its first turn.
    pub fn mcp_servers_connected(&self) -> bool {
        self.mcp_servers
            .iter()
            .all(|server| server.status == MCP_SERVER_CONNECTED)
    }

    /// Whether the session's tools are exactly `expected`, in any order.
    pub fn tools_are_exactly(&self, expected: &[&str]) -> bool {
        let mut have: Vec<&str> = self.tools.iter().map(String::as_str).collect();
        let mut want = expected.to_vec();
        have.sort_unstable();
        want.sort_unstable();
        have == want
    }
}

/// One content block of an assistant API message.
#[derive(Debug, Clone, PartialEq)]
pub enum AssistantContent {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// A thinking block. The CLI redacts it (empty text, signature only),
    /// so `text` is `None` unless a non-empty text arrived.
    Thinking {
        text: Option<String>,
    },
}

/// A `tool_result` block.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResultEvent {
    /// The call this answers; `None` when the line did not name it (the
    /// projection then closes the oldest open call).
    pub tool_use_id: Option<String>,
    /// The tool's result: a string or a list of content blocks.
    pub content: Value,
    pub is_error: bool,
    /// The tool never ran: a permission rule (a PreToolUse hook) refused
    /// it (`tool_result_meta[].non_execution_kind == "permission-rule"`).
    pub permission_denied: bool,
}

impl ToolResultEvent {
    /// The result's text: the string itself, or the `text` of every text
    /// block, joined by newlines. Other block kinds (images) carry no text.
    pub fn content_text(&self) -> String {
        match &self.content {
            Value::String(text) => text.clone(),
            Value::Array(blocks) => blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    }
}

/// `system/task_started`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStarted {
    pub task_id: String,
    pub tool_use_id: Option<String>,
    pub description: Option<String>,
    pub is_backgrounded: bool,
}

/// `system/task_notification`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskNotification {
    pub task_id: String,
    pub tool_use_id: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
}

/// One entry of `system/background_tasks_changed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTask {
    pub task_id: String,
    pub description: Option<String>,
}

/// `rate_limit_event.rate_limit_info`.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitInfo {
    pub status: RateLimitStatus,
    /// The window the headline `utilization` belongs to (`seven_day` …).
    pub limit_type: Option<String>,
    pub utilization: Option<f64>,
    /// Unix seconds.
    pub resets_at: Option<i64>,
    pub windows: Vec<RateLimitWindow>,
    pub using_overage: Option<bool>,
}

/// A rate-limit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitStatus {
    Allowed,
    AllowedWarning,
    Rejected,
    Other(String),
}

impl RateLimitStatus {
    pub fn parse(status: &str) -> Self {
        match status {
            "allowed" => Self::Allowed,
            "allowed_warning" => Self::AllowedWarning,
            "rejected" => Self::Rejected,
            other => Self::Other(other.to_string()),
        }
    }

    /// Whether the status is worth an admission warning: a warning, a
    /// rejection, or a status this vocabulary does not know.
    pub fn warrants_warning(&self) -> bool {
        matches!(self, Self::AllowedWarning | Self::Rejected | Self::Other(_))
    }
}

/// One named rate-limit window (`five_hour`, `seven_day` …).
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitWindow {
    pub name: String,
    pub utilization: Option<f64>,
    pub resets_at: Option<i64>,
}

/// `result`: the end of one user turn.
///
/// `usage` covers this turn only; `total_cost_usd` and `model_usage` are
/// cumulative for the process. `subtype` is deliberately absent: it says
/// `success` even for a failed turn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResultEvent {
    pub is_error: Option<bool>,
    pub terminal_reason: Option<String>,
    pub stop_reason: Option<String>,
    pub api_error_status: Option<u16>,
    pub result_text: Option<String>,
    /// An error result's reasons (`error_max_turns`,
    /// `error_max_budget_usd`, `error_during_execution` carry these and no
    /// `result`), and any the codec adds for a malformed turn end.
    pub errors: Vec<String>,
    pub usage: TokenCounts,
    pub total_cost_usd: Option<f64>,
    pub model_usage: Vec<ModelUsage>,
    pub permission_denials: Vec<PermissionDenial>,
    pub num_turns: Option<u32>,
    pub duration_ms: Option<u64>,
    /// The time the turn spent in API calls (#2304).
    pub duration_api_ms: Option<u64>,
    /// The ids of the user turns this turn consumed
    /// (`user_message_uuids`, else `user_message_uuid`): a turn's own and
    /// every one folded into it mid-turn. Empty when the CLI names none.
    pub user_turn_ids: Vec<String>,
}

/// Token counts: one turn's (`result.usage`) or cumulative (`modelUsage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenCounts {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenCounts {
    pub fn plus(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            output: self.output.saturating_add(other.output),
            cache_read: self.cache_read.saturating_add(other.cache_read),
            cache_write: self.cache_write.saturating_add(other.cache_write),
        }
    }
}

/// One model's cumulative usage in `result.modelUsage`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelUsage {
    pub model: String,
    pub tokens: TokenCounts,
    pub cost_usd: Option<f64>,
}

/// One entry of `result.permission_denials`: a tool call a permission
/// rule refused.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionDenial {
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: Value,
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
