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
    #[cfg(any(test, feature = "test-support"))]
    SubagentAwait {
        agent_id: String,
        status: String,
        reason: Option<String>,
        elapsed_ms: u64,
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
mod tests {
    use super::*;

    #[test]
    fn tool_call_round_trip() {
        let event = AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "call_1".into(),
            arguments: r#"{"command":"ls"}"#.into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn tool_result_round_trip() {
        let event = AuditEvent::ToolResult {
            call_id: "call_1".into(),
            tool: "bash".into(),
            is_error: false,
            content_tokens: 450,
            content_preview: "ok".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn llm_turn_start_round_trip() {
        let event = AuditEvent::LlmTurnStart {
            input_tokens_estimate: 45200,
            message_count: 34,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn llm_turn_end_round_trip() {
        let event = AuditEvent::LlmTurnEnd {
            input_tokens: 45200,
            output_tokens: 1830,
            stop_reason: "tool_use".into(),
            duration_ms: 4200,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn workflow_step_round_trip() {
        let event = AuditEvent::WorkflowStep {
            action: "check".into(),
            step_index: 3,
            step_key: "red".into(),
            step_label: "Ensure tests fail".into(),
            template_id: "feature".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn workflow_transition_round_trip() {
        let event = AuditEvent::WorkflowTransition {
            from_mode: "selector".into(),
            to_mode: "active".into(),
            template_id: Some("feature".into()),
            issue: Some(AuditIssue {
                number: 42,
                title: "Add auth".into(),
            }),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn context_pruned_round_trip() {
        let event = AuditEvent::ContextPruned {
            messages_dropped: 12,
            tool_results_collapsed: 0,
            tokens_before: 195_000,
            tokens_after: 142_000,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn subagent_spawned_round_trip() {
        let event = AuditEvent::SubagentSpawned {
            agent_id: "arch-review".into(),
            task_preview: "Review src/".into(),
            system_preview: "You are...".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn subagent_cmd_round_trip() {
        let event = AuditEvent::SubagentCmd {
            agent_id: "arch-review".into(),
            command: "get_messages_tail".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn guard_blocked_round_trip() {
        let event = AuditEvent::GuardBlocked {
            command_preview: "git commit".into(),
            guard_message: "Complete steps first".into(),
            before_step_key: "commit".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn error_round_trip() {
        let event = AuditEvent::Error {
            source: "tool".into(),
            tool: Some("bash".into()),
            message: "Command timed out".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn error_without_tool_round_trip() {
        let event = AuditEvent::Error {
            source: "provider".into(),
            tool: None,
            message: "rate limited".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn subagent_await_round_trip() {
        let event = AuditEvent::SubagentAwait {
            agent_id: "bookmarks-v1".into(),
            status: "idle".into(),
            reason: Some("completed".into()),
            elapsed_ms: 52000,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn subagent_await_null_reason_round_trip() {
        let event = AuditEvent::SubagentAwait {
            agent_id: "worker-1".into(),
            status: "timeout".into(),
            reason: None,
            elapsed_ms: 120000,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn envelope_round_trip() {
        let envelope = AuditEnvelope {
            ts: "2026-03-28T14:32:01.847Z".into(),
            session: "cli:my-feature".into(),
            turn: 7,
            event: AuditEvent::ToolCall {
                tool: "bash".into(),
                call_id: "call_abc".into(),
                arguments: r#"{"command":"test"}"#.into(),
            },
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let back: AuditEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope, back);
        // Verify flattened: "event" field should be at top level
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["event"], "tool_call");
        assert_eq!(val["ts"], "2026-03-28T14:32:01.847Z");
        assert_eq!(val["session"], "cli:my-feature");
        assert_eq!(val["turn"], 7);
        assert_eq!(val["tool"], "bash");
    }

    #[test]
    fn subagent_cmd_redacts_sk_token() {
        let event = AuditEvent::subagent_cmd(
            "arch-review".into(),
            "deploy --api-key=sk-abc123SECRETvalue stack",
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            !json.contains("sk-abc123SECRETvalue"),
            "sk- token should be redacted: {json}"
        );
        assert!(json.contains("[REDACTED]"), "should mark redaction: {json}");
        assert!(json.contains("deploy"), "non-secret token kept: {json}");
        assert!(json.contains("stack"), "non-secret token kept: {json}");
    }

    #[test]
    fn subagent_cmd_redacts_bearer_token() {
        let event = AuditEvent::subagent_cmd(
            "a".into(),
            "curl -H 'Authorization: Bearer tok_supersecret123'",
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            !json.contains("tok_supersecret123"),
            "bearer redacted: {json}"
        );
        assert!(json.contains("[REDACTED]"));
        assert!(json.contains("curl"), "non-secret token kept: {json}");
        assert!(
            json.contains("Authorization"),
            "non-secret token kept: {json}"
        );
    }

    #[test]
    fn subagent_cmd_redacts_api_key_env_assignment() {
        let event =
            AuditEvent::subagent_cmd("a".into(), "OPENAI_API_KEY=verysecretvalue123 run thing");
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            !json.contains("verysecretvalue123"),
            "env key redacted: {json}"
        );
        assert!(json.contains("[REDACTED]"));
        assert!(json.contains("run"), "non-secret token kept: {json}");
        assert!(json.contains("thing"), "non-secret token kept: {json}");
    }

    #[test]
    fn subagent_cmd_preserves_nonsensitive_command() {
        let event = AuditEvent::subagent_cmd("a".into(), "get_messages_tail --limit 5");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("get_messages_tail --limit 5"), "kept: {json}");
        assert!(!json.contains("[REDACTED]"));
    }

    #[test]
    fn subagent_cmd_passes_through_plain_text() {
        let AuditEvent::SubagentCmd { command, .. } =
            AuditEvent::subagent_cmd("a1".into(), "ls -la /tmp")
        else {
            panic!("expected SubagentCmd");
        };
        assert_eq!(command, "ls -la /tmp");
    }

    #[test]
    fn subagent_cmd_scrubs_flag_api_key() {
        let AuditEvent::SubagentCmd { command, .. } =
            AuditEvent::subagent_cmd("a1".into(), "tool --api-key=sk-livedeadbeef0001")
        else {
            panic!("expected SubagentCmd");
        };
        assert!(!command.contains("sk-livedeadbeef0001"), "out={command}");
        assert!(command.contains("[REDACTED]"));
    }

    #[test]
    fn provider_error_round_trip() {
        use crate::domain::provider_error::ProviderErrorClass;
        let event = AuditEvent::ProviderError {
            provider: "fireworks".into(),
            class: ProviderErrorClass::Client,
            http_status: Some(400),
            body: r#"{"error":{"type":"invalid_request_error","message":"bad"}}"#.into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["event"], "provider_error");
        // Typed class still serializes to the stable snake_case wire string.
        assert_eq!(val["class"], "client");
    }

    #[test]
    fn provider_error_null_status_round_trip() {
        use crate::domain::provider_error::ProviderErrorClass;
        let event = AuditEvent::ProviderError {
            provider: "anthropic".into(),
            class: ProviderErrorClass::Network,
            http_status: None,
            body: "connection reset".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn provider_error_keeps_full_untruncated_body() {
        use crate::domain::provider_error::ProviderErrorClass;
        use crate::domain::redaction::redact_secrets;
        // A body far longer than any TUI/preview cap (200 chars) must survive
        // intact in the persisted audit record (#937 AC1/AC5). A planted secret
        // is woven into the long body so this test would also break if the
        // constructor ever stopped redacting OR started truncating: we assert
        // the persisted body equals the *redacted* original exactly (so any
        // truncation cap wired into the path would change the length and fail),
        // and that the secret is gone while the bulk filler survives.
        let secret = "sk-abc123SECRETvalue0001";
        let long_message = "x".repeat(5000);
        let body = format!(r#"{{"error":{{"key":"{secret}","message":"{long_message}"}}}}"#);
        let AuditEvent::ProviderError {
            body: persisted, ..
        } = AuditEvent::provider_error("fireworks", &ProviderErrorClass::Client, Some(400), &body)
        else {
            panic!("expected ProviderError");
        };
        // Exact match against the redacted (not truncated) original: this is the
        // identity that a truncation regression would break.
        assert_eq!(
            persisted.as_str(),
            redact_secrets(&body),
            "persisted body must be the full, redacted-but-untruncated original"
        );
        assert!(
            persisted.len() > 4000,
            "body kept whole: {}",
            persisted.len()
        );
        assert!(persisted.contains(&long_message), "filler survived whole");
        assert!(!persisted.contains(secret), "planted secret was redacted");
    }

    #[test]
    fn provider_error_redacts_planted_secret() {
        use crate::domain::provider_error::ProviderErrorClass;
        let body = r#"{"error":{"message":"rejected request with key sk-abc123SECRETvalue here"}}"#;
        let event =
            AuditEvent::provider_error("openai", &ProviderErrorClass::Auth, Some(401), body);
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            !json.contains("sk-abc123SECRETvalue"),
            "planted secret must be redacted: {json}"
        );
        assert!(json.contains("[REDACTED]"), "should mark redaction: {json}");
        assert!(
            json.contains("rejected request"),
            "non-secret text kept: {json}"
        );
        assert!(json.contains("openai"), "provider kept: {json}");
        assert!(json.contains("auth"), "class kept: {json}");
    }

    #[test]
    fn content_preview_short_passthrough() {
        assert_eq!(content_preview("hello", 200), "hello");
    }

    #[test]
    fn content_preview_truncates_at_200() {
        let long = "x".repeat(500);
        let preview = content_preview(&long, 200);
        assert!(preview.chars().count() <= 200);
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn content_preview_empty() {
        assert_eq!(content_preview("", 200), "");
    }

    #[test]
    fn content_preview_exact_boundary() {
        let exact = "x".repeat(200);
        assert_eq!(content_preview(&exact, 200), exact);
    }
}
