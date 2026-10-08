//! Audit event domain types.
//!
//! Pure domain types for the append-only audit log. No I/O — serialisation
//! and file writing live in `infrastructure::persistence::audit_log`, and the
//! sink port the agent loop emits through is
//! `application::audit::ports::AuditSink` (#1960).

use serde::{Deserialize, Serialize};

/// Issue reference for workflow transition events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditIssue {
    pub number: u64,
    pub title: String,
}

/// A single audit event. Engine-authored, never fabricated by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AuditEvent {
    RequestObserved {
        observation: Box<crate::domain::inference::events::request_observation::RequestObservation>,
    },
    ToolCall {
        tool: String,
        call_id: String,
        /// What the tool ran with.
        arguments: String,
        /// What the model sent, when it differs (invalid arguments the
        /// harness replaced, #2123): kept for diagnosis (#2150).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_arguments: Option<String>,
    },
    ToolResult {
        call_id: String,
        tool: String,
        is_error: bool,
        content_tokens: usize,
        content_preview: String,
        /// How long the call took, policy and approval waits included
        /// (#2150).
        #[serde(default)]
        duration_ms: u64,
        /// The arguments the tool ran with, in bytes (#2150).
        #[serde(default)]
        argument_bytes: usize,
        /// The result the tool answered, in bytes (#2150).
        #[serde(default)]
        content_bytes: usize,
    },
    /// The log reached its size cap: nothing more is written (#2150).
    LogCapped { cap_bytes: u64 },
    /// One swarm board op (#2303): ids, kinds, durations and sizes only.
    SwarmOp(super::swarm::BoardOpObservation),
    /// `dropped` swarm board ops went unrecorded since the last `swarm_op`
    /// written: the log's write gate stayed busy past the bound a record
    /// waits for it (#2303). Written just before the next `swarm_op`, in
    /// the same write.
    SwarmOpsDropped { dropped: u64 },
    /// `dropped` run summaries went unwritten (#2313 final review): the
    /// gate stayed busy past the summary's bound, or they were held past
    /// the bound before the session's log opened. Counted apart from the
    /// `swarm_op` records, and written before the next record.
    SwarmRunSummaryDropped { dropped: u64 },
    /// A swarm run's `swarm_op` records, folded (#2313): written once by
    /// the coordinator's harness when the run settles.
    SwarmRunSummary(super::swarm::SwarmRunSummary),
    LlmTurnStart {
        input_tokens_estimate: usize,
        message_count: usize,
    },
    LlmTurnEnd {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage_source: Option<String>,
        input_tokens: usize,
        output_tokens: usize,
        stop_reason: String,
        duration_ms: u64,
        /// The share of `input_tokens` the provider served from its prompt
        /// cache (#2348): Anthropic `cache_read_input_tokens`, OpenAI chat
        /// `prompt_tokens_details.cached_tokens`, Codex/Responses
        /// `input_tokens_details.cached_tokens`. Absent when the provider
        /// reported none (or no usage at all), and in logs written before it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cached_input_tokens: Option<usize>,
        /// The share of `input_tokens` the provider wrote to its prompt
        /// cache (#2348 review L3): Anthropic `cache_creation_input_tokens`,
        /// the signal of a prefix rewrite. Absent when the provider reported
        /// none, and in logs written before it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_write_tokens: Option<usize>,
    },
    #[cfg(any(test, feature = "test-support"))]
    WorkflowStep {
        action: String,
        step_index: usize,
        step_key: String,
        step_label: String,
        template_id: String,
    },
    #[cfg(any(test, feature = "test-support"))]
    WorkflowTransition {
        from_mode: String,
        to_mode: String,
        template_id: Option<String>,
        issue: Option<AuditIssue>,
    },
    /// The emergency ladder's record (#2414): no watermark cut brought the
    /// request under the ceiling, so the ladder stubbed, then dropped, the
    /// oldest messages; or the pinned set alone is over the ceiling.
    ContextPruned {
        messages_dropped: usize,
        tokens_before: usize,
        tokens_after: usize,
        /// True when the ceiling could not be met — the pinned/exempt set
        /// alone exceeds the budget even after full demotion (#1044).
        #[serde(default)]
        budget_unmet: bool,
        /// Messages the ladder's first rung collapsed to recall stubs this
        /// prune (#2214); absent from logs written before it, read as 0.
        #[serde(default)]
        ladder_stubbed: usize,
        /// The ceiling in force, in provider tokens (the unit of
        /// `max_context_tokens`, before the ladder converts it to estimate
        /// units at the observed scale): the lowest of the configured
        /// budget, the model's window and a swarm member's cap (#2342);
        /// absent from logs written before it, read as 0.
        #[serde(default)]
        ceiling_tokens: usize,
        /// The emergency ladder ran (#2404): no cut brought the request
        /// under the ceiling. A watermark cut is never a `context_pruned`
        /// record (it is a `context_cut`); false when only the budget was
        /// unmet (on every request while the tool definitions exceed three
        /// quarters of it, so the messages keep the quarter floor), and in
        /// logs written before it.
        #[serde(default)]
        watermark_fallback: bool,
    },
    /// A watermark cut (#2404).
    ContextCut(ContextCutRecord),
    /// A watermark cut was due but not made (#2404).
    ContextCutSkipped(ContextCutSkippedRecord),
    #[cfg(any(test, feature = "test-support"))]
    SubagentSpawned {
        agent_id: String,
        task_preview: String,
        system_preview: String,
    },
    #[cfg(any(test, feature = "test-support"))]
    SubagentCmd { agent_id: String, command: String },
    #[cfg(any(test, feature = "test-support"))]
    GuardBlocked {
        command_preview: String,
        guard_message: String,
        before_step_key: String,
    },
    Error {
        source: String,
        tool: Option<String>,
        message: String,
        /// Where a panic happened, `file:line:column` (#2192).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location: Option<String>,
    },
    /// A provider call that ultimately failed (after any retries) on a turn.
    ///
    /// Captures the *full*, untruncated error body so it survives past the
    /// TUI line-truncation that otherwise loses it (#937). Persisted once per
    /// terminal failure (not per retry). The `body` is a
    /// [`crate::domain::redaction::Redacted`] newtype whose only text
    /// constructors scrub secrets, so redaction-before-persistence is enforced
    /// by the type system: building this variant directly cannot bypass it
    /// (#939 review). `class` is the typed [`ProviderErrorClass`] so readers
    /// match on the enum instead of re-parsing a string.
    ProviderError {
        provider: String,
        class: crate::domain::inference::services::provider_error::ProviderErrorClass,
        http_status: Option<u16>,
        body: crate::domain::redaction::Redacted,
    },
}

/// Envelope wrapper for a single audit log line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEnvelope {
    pub ts: String,
    /// The same moment in Unix milliseconds (#2150).
    #[serde(default)]
    pub unix_ms: u64,
    /// The writing process (#2150).
    #[serde(default)]
    pub pid: u32,
    /// The host the process runs on (#2161): a container's own name, so
    /// records from containers, whose pids repeat, are told apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub session: String,
    /// A sub-agent's parent session (#2150).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The agent turn the record is filed under; `null` for a record
    /// written outside any turn's knowledge (a `swarm_op`, #2303).
    pub turn: Option<u32>,
    #[serde(flatten)]
    pub event: AuditEvent,
}

/// The watermark marks a cut was planned under (#2404), in **estimate
/// units**: the harness's own token estimate of the whole request
/// (messages and tool definitions), the unit the planner works in. The
/// configured marks and the ceiling are in provider tokens; they are
/// converted to estimate units at the provider-observed scale
/// (`estimate_scale_permille`), and a count here times that scale over
/// 1000 is in provider tokens again. `high_tokens` and `low_tokens` are as
/// the ceiling (`ceiling_estimate_tokens`) left them.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CutMarks {
    pub high_tokens: usize,
    pub low_tokens: usize,
    /// The pruning ceiling in estimate units (unlike `context_pruned`'s
    /// `ceiling_tokens`, which is in provider tokens).
    pub ceiling_estimate_tokens: usize,
    pub ceiling_lowered_marks: bool,
    /// The provider-observed scale, per mille (1000: none observed).
    pub estimate_scale_permille: u32,
}

/// What a watermark cut did (#2404): counts, ids and kinds, never content.
/// Token counts are in estimate units, as [`CutMarks`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCutRecord {
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub marks: CutMarks,
    /// The messages the cut archived, a previous cut's stub among them.
    pub messages_archived: usize,
    /// The messages it kept in place (its new stub not counted).
    pub messages_kept: usize,
    /// The archive index `recall` reaches (`archive`, `archive:2`...);
    /// `null` when nothing retains context or the write failed.
    pub archive_id: Option<String>,
    /// How the kept set compares with the low mark: `within_low`,
    /// `newest_exchange_over_low` (the newest exchange, kept whole, took
    /// it over) or `head_over_low` (the pinned head alone is over it).
    pub fill: crate::domain::conversation::services::watermark::Fill,
}

/// Why a due watermark cut was not made (#2404), adjacently tagged:
/// `{"kind": "no_boundary"}`, or `{"kind": "saving_too_small", "detail":
/// {"saving_tokens": .., "needed_tokens": ..}}`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum CutSkipReason {
    /// The conversation has no user message: no head to keep.
    NoUserMessage,
    /// Everything is pinned or a previous stub.
    NothingArchivable,
    /// No exchange boundary falls after something archivable: a single
    /// exchange follows the head (one huge result, or one batch of
    /// parallel calls).
    NoBoundary,
    /// The cut would save `saving_tokens`, under the `needed_tokens` a cut
    /// must save (estimate units).
    SavingTooSmall {
        saving_tokens: usize,
        needed_tokens: usize,
    },
}

/// A due watermark cut that was not made (#2404): the request's size (in
/// estimate units), the marks and why; counts and kinds only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCutSkippedRecord {
    pub tokens: usize,
    pub marks: CutMarks,
    pub reason: CutSkipReason,
}

impl AuditEvent {
    /// Build a [`AuditEvent::ProviderError`] carrying the full, untruncated
    /// provider error body, with secrets scrubbed before persistence (#937).
    ///
    /// The `body` is the complete error string built by the provider client
    /// (e.g. the embedded HTTP `.text()`), not a TUI preview — this is the
    /// whole point: the audit record must retain what the TUI throws away.
    /// It is routed through [`crate::domain::redaction::redact_secrets`] so
    /// any API key echoed back in the error never lands on disk.
    pub fn provider_error(
        provider: impl Into<String>,
        class: &crate::domain::inference::services::provider_error::ProviderErrorClass,
        http_status: Option<u16>,
        body: &str,
    ) -> Self {
        AuditEvent::ProviderError {
            provider: provider.into(),
            class: class.clone(),
            http_status,
            body: crate::domain::redaction::Redacted::new(body),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AuditEvent {
    /// Build a [`AuditEvent::SubagentCmd`], scrubbing known secret shapes from
    /// the command string before it is persisted.
    ///
    /// Subagent command lines can carry credentials or PII as arguments
    /// (`sk-...` tokens, `Bearer <tok>`, `--api-key=...`, `*_API_KEY=...`).
    /// Persisting them verbatim would be a leak risk (#790), so the command is
    /// passed through the shared [`crate::domain::redaction::redact_secrets`],
    /// which replaces only the secret-bearing spans with `[REDACTED]`, leaving
    /// the rest of the command intact and useful.
    ///
    /// The [`AuditEvent::SubagentCmd`] variant itself is currently
    /// test-support-only (no production code emits it yet); this constructor is
    /// the redacting entry point any future production emitter must route
    /// through. The redaction helper lives in
    /// [`crate::domain::redaction`] and compiles into release builds, so wiring
    /// up an emitter cannot accidentally bypass it.
    pub fn subagent_cmd(agent_id: String, command: &str) -> Self {
        AuditEvent::SubagentCmd {
            agent_id,
            command: crate::domain::redaction::redact_secrets(command),
        }
    }
}

/// Generate a content preview capped at `max_chars` characters.
///
/// Truncates at a character boundary and appends "..." when truncated (the
/// ellipsis counts toward the budget). Bounded-scan core in [`crate::domain::text`].
pub fn content_preview(content: &str, max_chars: usize) -> String {
    crate::domain::text::truncate_chars(content, max_chars, max_chars.saturating_sub(3), "...")
        .into_owned()
}

/// A failed tool's preview: its first `head_chars` and last `tail_chars`
/// characters, naming what lies between (#2159). A failure's cause (a
/// traceback, stderr's last lines) is usually at the end, which a
/// start-only preview cuts off.
pub fn error_preview(content: &str, head_chars: usize, tail_chars: usize) -> String {
    let total = content.chars().count();
    if total <= head_chars.saturating_add(tail_chars) {
        return content.to_string();
    }
    let head_end = content
        .char_indices()
        .nth(head_chars)
        .map_or(content.len(), |(index, _)| index);
    let tail_start = content
        .char_indices()
        .nth(total - tail_chars)
        .map_or(content.len(), |(index, _)| index);
    debug_assert!(head_end <= tail_start);
    let omitted = total - head_chars - tail_chars;
    format!(
        "{}\n[... {omitted} chars omitted ...]\n{}",
        &content[..head_end],
        &content[tail_start..]
    )
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
