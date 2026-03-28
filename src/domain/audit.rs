//! Audit event domain types.
//!
//! Pure domain types for the append-only audit log. No I/O — serialisation
//! and file writing live in `infrastructure::persistence::audit_log`.

use serde::{Deserialize, Serialize};

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
    WorkflowStep {
        action: String,
        step_index: usize,
        step_key: String,
        step_label: String,
        template_id: String,
    },
    WorkflowTransition {
        from_mode: String,
        to_mode: String,
        template_id: Option<String>,
        issue: Option<(u64, String)>,
    },
    ContextPruned {
        messages_dropped: usize,
        tool_results_collapsed: usize,
        tokens_before: usize,
        tokens_after: usize,
    },
    SubagentSpawned {
        agent_id: String,
        task_preview: String,
        system_preview: String,
    },
    SubagentCmd {
        agent_id: String,
        command: String,
    },
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

/// Generate a content preview capped at `max_chars` characters.
///
/// Truncates at a character boundary and appends "..." when truncated.
pub fn content_preview(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        content.to_string()
    } else {
        let truncated: String = content.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
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
            issue: Some((42, "Add auth".into())),
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
