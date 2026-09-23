use super::*;
use serde_json::json;

/// Test-local sanitizer: protocol must not depend on `interface::ansi`.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && *c != '\u{200b}')
        .collect()
}

#[test]
fn admission_warnings_accept_only_typed_sanitized_nonempty_entries() {
    let snap = parse_get_state(
        &json!({"admissionWarnings": [
            {"code":"admission_binding_missing","slot":"api\u{1b}[31m","message":"not broker-gated\u{1b}"},
            {"code":"other","slot":"ignored","message":"ignored"},
            {"code":"admission_binding_missing","slot":"","message":"ignored"}
        ]}),
        &sanitize,
    );
    assert_eq!(
        snap.admission_warnings,
        vec![AdmissionBindingWarning {
            slot: "api[31m".into(),
            message: "not broker-gated".into(),
        }]
    );
    assert!(parse_get_state(&json!({}), &sanitize)
        .admission_warnings
        .is_empty());
}

#[test]
fn malformed_warning_does_not_discard_valid_siblings() {
    let snap = parse_get_state(
        &json!({"admissionWarnings":[
            {"code":"admission_binding_missing","slot":"provider-a","message":"not broker-gated"},
            {"code":"admission_binding_missing","slot":42,"message":"invalid"},
            {"code":"admission_binding_missing","slot":"provider-b","message":"not broker-gated"}
        ]}),
        &sanitize,
    );
    assert_eq!(
        snap.admission_warnings
            .iter()
            .map(|w| w.slot.as_str())
            .collect::<Vec<_>>(),
        vec!["provider-a", "provider-b"]
    );
}

#[test]
fn warning_authority_requires_complete_valid_bounded_array() {
    let warning =
        json!({"code":"admission_binding_missing","slot":"a","message":"not broker-gated"});
    for value in [
        json!({}),
        json!({"admissionWarnings":null}),
        json!({"admissionWarnings":[warning, {"code":"unknown","slot":"b","message":"x"}]}),
        json!({"admissionWarnings":vec![warning.clone();65]}),
    ] {
        assert!(
            !parse_get_state(&value, &sanitize).admission_warnings_authoritative,
            "{value}"
        );
    }
    assert!(
        parse_get_state(&json!({"admissionWarnings":[]}), &sanitize)
            .admission_warnings_authoritative
    );
    assert!(
        parse_get_state(&json!({"admissionWarnings":[warning]}), &sanitize)
            .admission_warnings_authoritative
    );
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
fn malformed_footer_field_does_not_discard_valid_siblings() {
    let fields = parse_get_state_footer(
        &json!({"model":42,"maxContextTokens":200_000,"effort":"high"}),
        &sanitize,
    );
    assert_eq!(fields.model, None);
    assert_eq!(fields.max_context_tokens, Some(200_000));
    assert_eq!(fields.effort.as_deref(), Some("high"));
    let fields = parse_get_state_footer(
        &json!({"model":"provider/model","maxContextTokens":"invalid","effort":"low"}),
        &sanitize,
    );
    assert_eq!(fields.model.as_deref(), Some("provider/model"));
    assert_eq!(fields.max_context_tokens, None);
    assert_eq!(fields.effort.as_deref(), Some("low"));
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
        snap.effort_levels.as_deref(),
        Some(&["none", "low", "medium", "high", "xhigh"].map(String::from)[..])
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
    assert_eq!(
        snap.effort_levels,
        Some(vec!["low".into(), "high".into(), "".into()])
    );
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
