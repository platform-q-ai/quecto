//! Audit event domain types.
//!
//! Pure domain types for the append-only audit log. No I/O — serialisation
//! and file writing live in `infrastructure::persistence::audit_log`.

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
    ToolCall {
        tool: String,
        call_id: String,
        arguments: String,
    },
    ToolResult {
        call_id: String,
        tool: String,
        is_error: bool,
        content_tokens: usize,
        content_preview: String,
    },
    LlmTurnStart {
        input_tokens_estimate: usize,
        message_count: usize,
    },
    LlmTurnEnd {
        input_tokens: usize,
        output_tokens: usize,
        stop_reason: String,
        duration_ms: u64,
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
    ContextPruned {
        messages_dropped: usize,
        tool_results_collapsed: usize,
        tokens_before: usize,
        tokens_after: usize,
        /// True when the ceiling could not be met — the pinned/exempt set
        /// alone exceeds the budget even after full demotion (#1044).
        #[serde(default)]
        budget_unmet: bool,
    },
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
        class: crate::domain::provider_error::ProviderErrorClass,
        http_status: Option<u16>,
        body: crate::domain::redaction::Redacted,
    },
}

/// Envelope wrapper for a single audit log line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEnvelope {
    pub ts: String,
    pub session: String,
    pub turn: u32,
    #[serde(flatten)]
    pub event: AuditEvent,
}

/// Trait for audit event sinks. Implemented by AuditLog in infrastructure.
///
/// This trait lives in the domain layer so the application layer (agent_loop)
/// can depend on it without importing infrastructure types.
///
/// Uses boxed futures instead of `async fn` for dyn-compatibility.
pub trait AuditSink: Send + Sync {
    fn emit(
        &self,
        turn: u32,
        event: AuditEvent,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    >;
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
        class: &crate::domain::provider_error::ProviderErrorClass,
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

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
