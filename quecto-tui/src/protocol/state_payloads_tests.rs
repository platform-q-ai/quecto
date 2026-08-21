use super::*;
use serde_json::json;

/// Test-local sanitizer: protocol must not depend on `interface::ansi`.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && *c != '\u{200b}')
        .collect()
}

#[test]
fn parse_get_state_footer_extracts_model_window_and_effort() {
    let fields = parse_get_state_footer(
        &json!({
            "model": "openai/gpt-5",
            "maxContextTokens": 200_000u64,
            "effort": "high",
        }),
        &sanitize,
    );
    assert_eq!(fields.model.as_deref(), Some("openai/gpt-5"));
    assert_eq!(fields.max_context_tokens, Some(200_000));
    assert_eq!(fields.effort.as_deref(), Some("high"));
}

#[test]
fn parse_get_state_footer_treats_missing_and_null_effort_as_default() {
    let missing = parse_get_state_footer(&json!({"model": "m"}), &sanitize);
    assert_eq!(missing.effort, None);
    let null = parse_get_state_footer(&json!({"effort": null}), &sanitize);
    assert_eq!(null.effort, None);
}

#[test]
fn parse_get_state_footer_strips_control_characters() {
    let fields = parse_get_state_footer(
        &json!({"model": "bad\u{1b}[31mm", "effort": "hi\u{200b}gh"}),
        &sanitize,
    );
    assert_eq!(fields.model.as_deref(), Some("bad[31mm"));
    assert_eq!(fields.effort.as_deref(), Some("high"));
}

#[test]
fn real_harness_get_state_contract_drives_effort_and_resume() {
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../../quecto-agentic-harness/tests/fixtures/get_state_effort_contract.json"
    ))
    .unwrap();
    let snap = parse_get_state(&data, &sanitize);
    assert_eq!(
        snap.effort_levels,
        vec!["none", "low", "medium", "high", "xhigh"]
    );
    assert_eq!(snap.footer.effort.as_deref(), Some("high"));
    assert_eq!(snap.session_key.as_deref(), Some("cli:contract-worker"));
}

#[test]
fn parse_get_state_collects_effort_levels_and_session_key() {
    let snap = parse_get_state(
        &json!({
            "model": "m",
            "effortLevels": ["low", "high", ""],
            "sessionKey": "cli:worker",
            "workflow": {"mode": "active"},
        }),
        &sanitize,
    );
    // Empty strings after sanitize are retained (historical parity).
    assert_eq!(snap.effort_levels, vec!["low", "high", ""]);
    assert_eq!(snap.session_key.as_deref(), Some("cli:worker"));
    assert!(snap.workflow.is_some());
}

#[test]
fn parse_set_effort_level_reads_echoed_effort() {
    assert_eq!(
        parse_set_effort_level(&json!({"effort": "max"}), &sanitize).as_deref(),
        Some("max")
    );
    assert_eq!(parse_set_effort_level(&json!({}), &sanitize), None);
}

#[test]
fn parse_set_model_id_reads_echoed_model() {
    assert_eq!(
        parse_set_model_id(&json!({"model": "x"}), &sanitize).as_deref(),
        Some("x")
    );
    assert_eq!(parse_set_model_id(&json!({}), &sanitize), None);
}

#[test]
fn parse_resume_session_name_defaults_when_missing() {
    assert_eq!(parse_resume_session_name(&json!({"session": "abc"})), "abc");
    assert_eq!(parse_resume_session_name(&json!({})), "session");
}
