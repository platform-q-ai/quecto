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
        budget_unmet: false,
    };
    let json = serde_json::to_string(&event).unwrap();
    let back: AuditEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(event, back);
}

#[test]
fn context_pruned_round_trip_preserves_unmet_budget() {
    // #1044: a serializer that dropped the field would still round-trip
    // the `false` case (via #[serde(default)]); only `true` catches it.
    let event = AuditEvent::ContextPruned {
        messages_dropped: 0,
        tool_results_collapsed: 0,
        tokens_before: 300,
        tokens_after: 300,
        budget_unmet: true,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"budget_unmet\":true"), "got: {json}");
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
    let event = AuditEvent::subagent_cmd("a".into(), "OPENAI_API_KEY=verysecretvalue123 run thing");
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
    let event = AuditEvent::provider_error("openai", &ProviderErrorClass::Auth, Some(401), body);
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
