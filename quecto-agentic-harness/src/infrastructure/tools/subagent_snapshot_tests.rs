use super::*;

#[test]
fn slim_get_state_snapshot_validation_rejects_malformed_projections() {
    let command = r#"{"type":"get_state"}"#;
    let base = serde_json::json!({
        "type": "response", "command": "get_state",
        "data": {
            "state": "runningTool", "effort": null, "model": "mock",
            "sessionKey": "cli:dog-story-writer",
            "progress": { "state": "active", "reason": "busy" },
            "generation": 7,
            "workflow": {
                "activeTemplate": { "id": "bugfix" },
                "currentStep": {
                    "index": 1, "key": "red", "label": "RED",
                    "phase": "RED", "done": false
                }
            }
        }
    });
    assert!(response_is_valid_answer(&base, command));

    for pointer in [
        "/data/progress/state",
        "/data/progress/reason",
        "/data/workflow/activeTemplate/id",
        "/data/workflow/currentStep/index",
        "/data/workflow/currentStep/key",
        "/data/workflow/currentStep/label",
        "/data/workflow/currentStep/phase",
        "/data/workflow/currentStep/done",
    ] {
        let mut malformed = base.clone();
        malformed.pointer_mut(pointer).unwrap().take();
        assert!(
            !response_is_valid_answer(&malformed, command),
            "missing required slim field {pointer} must be rejected"
        );
    }

    for (pointer, value) in [
        ("/data/progress/extra", serde_json::json!(true)),
        ("/data/workflow/extra", serde_json::json!(true)),
        (
            "/data/workflow/activeTemplate/name",
            serde_json::json!("Bugfix"),
        ),
        ("/data/workflow/currentStep/extra", serde_json::json!(true)),
    ] {
        let mut malformed = base.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        malformed
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(key.into(), value);
        assert!(
            !response_is_valid_answer(&malformed, command),
            "unexpected slim field {pointer} must be rejected"
        );
    }

    for replacement in [
        serde_json::json!({"unchanged": true}),
        serde_json::json!({"unchanged": true, "generation": 7, "state": "idle"}),
        serde_json::json!([]),
    ] {
        let mut malformed = base.clone();
        malformed["data"] = replacement;
        assert!(!response_is_valid_answer(&malformed, command));
    }
}

#[test]
fn older_get_state_snapshot_finalizes_to_unchanged_at_caller_cursor() {
    let snapshot = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "data": {
            "state": "runningTool",
            "effort": null,
            "model": "mock",
            "sessionKey": "cli:dog-story-writer",
            "progress": { "state": "active", "reason": "busy" },
            "generation": 7
        }
    });
    let command = r#"{"type":"get_state","since":8}"#;
    assert!(response_is_valid_answer(&snapshot, command));
    let finalized = finalize_snapshot_answer(snapshot.to_string(), snapshot, command);
    let json: serde_json::Value = serde_json::from_str(&finalized).unwrap();
    assert_eq!(
        json["data"],
        serde_json::json!({ "unchanged": true, "generation": 8 })
    );
}

#[test]
fn older_unchanged_get_state_snapshot_finalizes_to_caller_cursor() {
    let snapshot = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "data": { "unchanged": true, "generation": 7 }
    });
    let command = r#"{"type":"get_state","since":8}"#;
    assert!(response_is_valid_answer(&snapshot, command));
    let finalized = finalize_snapshot_answer(snapshot.to_string(), snapshot.clone(), command);
    let json: serde_json::Value = serde_json::from_str(&finalized).unwrap();
    assert_eq!(
        json["data"],
        serde_json::json!({ "unchanged": true, "generation": 8 })
    );
    assert!(!response_is_valid_answer(
        &snapshot,
        r#"{"type":"get_state","since":6}"#
    ));
    let matching = r#"{"type":"get_state","since":7}"#;
    assert!(response_is_valid_answer(&snapshot, matching));
    let original = snapshot.to_string();
    assert_eq!(
        finalize_snapshot_answer(original.clone(), snapshot, matching),
        original
    );
}

fn production_state() -> crate::interface::cli::protocol::SessionState {
    serde_json::from_value(serde_json::json!({
        "model": "mock", "generation": 7, "isStreaming": true,
        "sessionKey": "session", "messageCount": 0,
        "pendingMessageCount": 0, "maxContextTokens": 0
    }))
    .unwrap()
}

fn production_response(state: &crate::interface::cli::protocol::SessionState) -> serde_json::Value {
    serde_json::to_value(crate::interface::cli::protocol::AgentEvent::ok(
        None,
        "get_state",
        Some(state.slim_projection()),
    ))
    .unwrap()
}

#[test]
fn real_production_projection_is_accepted_by_snapshot_consumer() {
    let response = production_response(&production_state());
    assert!(
        response_is_valid_answer(&response, r#"{"type":"get_state"}"#),
        "{response}"
    );
}

#[test]
fn real_projection_optional_variants_cursors_and_target_isolation() {
    for suspended in [false, true] {
        for receipts in [false, true] {
            for admission in [false, true] {
                for workflow in [false, true] {
                    let mut state = production_state();
                    state.automatic_turns_suspended = suspended;
                    state.repeated_failure_notifications = if suspended { 3 } else { 0 };
                    if receipts {
                        state.control_receipts = serde_json::from_value(serde_json::json!([
                            {"id":"control-1", "command":"pause", "status":"queued"}
                        ]))
                        .unwrap();
                    }
                    if admission {
                        state.execution = Some(Default::default());
                        state.execution.as_mut().unwrap().admission =
                            Some(serde_json::from_value(admission_fixture()).unwrap());
                    }
                    if workflow {
                        state.workflow = Some(serde_json::json!({"activeTemplate":{"id":"bugfix"},
                            "currentStep":{"index":1,"key":"red","label":"RED","phase":"test","done":false}}));
                    }
                    let response = production_response(&state);
                    assert_eq!(response["data"].get("admission").is_some(), admission);
                    for since in [6, 7, 8] {
                        let command =
                            serde_json::json!({"type":"get_state","since":since}).to_string();
                        assert!(response_is_valid_answer(&response, &command), "{response}");
                        let final_value: serde_json::Value =
                            serde_json::from_str(&finalize_snapshot_answer(
                                response.to_string(),
                                response.clone(),
                                &command,
                            ))
                            .unwrap();
                        if since >= 7 {
                            assert_eq!(
                                final_value["data"],
                                serde_json::json!({"unchanged":true,"generation":since})
                            );
                        } else {
                            assert_eq!(final_value, response);
                        }
                    }
                    for command in [
                        r#"{"type":"get_state","agent_id":"nested"}"#,
                        r#"{"type":"get_state","count":2}"#,
                        r#"{"type":"get_state","since":"7"}"#,
                        r#"{"type":"get_messages"}"#,
                    ] {
                        assert!(!response_is_valid_answer(&response, command));
                    }
                    let mut correlated = response.clone();
                    correlated["id"] = serde_json::json!("request-1");
                    assert!(!response_is_valid_answer(
                        &correlated,
                        r#"{"type":"get_state"}"#
                    ));
                }
            }
        }
    }
}

fn admission_fixture() -> serde_json::Value {
    serde_json::json!({"waiting":1,"admitted":2,"longestWaitSeconds":4,
        "groups":[{"group":"provider", "cooldown":{"state":"until","remainingSeconds":3},"lastRefusal":"capacity"}],
        "counters":{"completed":1,"refused":2,"cancelled":3,"abandoned":4},"hidden":0,"revision":9})
}

#[test]
fn real_projection_rejects_malformed_and_unknown_control_admission_shapes() {
    let mut base = production_response(&production_state());
    base["data"]["controlReceipts"] =
        serde_json::json!([{"id":"c","command":"pause","status":"queued"}]);
    base["data"]["admission"] = admission_fixture();
    let command = r#"{"type":"get_state"}"#;
    assert!(response_is_valid_answer(&base, command));
    for (pointer, value) in [
        ("/data/effort", serde_json::json!(4)),
        ("/data/effortLevels", serde_json::json!([3])),
        ("/data/sessionKey", serde_json::json!([])),
        ("/data/automaticTurnsSuspended", serde_json::json!("false")),
        ("/data/repeatedFailureNotifications", serde_json::json!(-1)),
        ("/data/controlReceipts", serde_json::json!({})),
        ("/data/controlReceipts/0/id", serde_json::Value::Null),
        (
            "/data/controlReceipts/0/status",
            serde_json::json!("invented"),
        ),
        ("/data/admission", serde_json::Value::Null),
        ("/data/admission/waiting", serde_json::json!(-1)),
        ("/data/admission/groups", serde_json::json!({})),
        (
            "/data/admission/groups/0/cooldown/state",
            serde_json::json!("invented"),
        ),
        (
            "/data/admission/groups/0/cooldown/remainingSeconds",
            serde_json::json!("3"),
        ),
        (
            "/data/admission/counters/completed",
            serde_json::Value::Null,
        ),
    ] {
        let mut malformed = base.clone();
        *malformed.pointer_mut(pointer).unwrap() = value;
        assert!(
            !response_is_valid_answer(&malformed, command),
            "{pointer}: {malformed}"
        );
    }
    for pointer in [
        "/data",
        "/data/controlReceipts/0",
        "/data/admission",
        "/data/admission/groups/0",
        "/data/admission/groups/0/cooldown",
        "/data/admission/counters",
    ] {
        let mut malformed = base.clone();
        malformed
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::json!(true));
        assert!(!response_is_valid_answer(&malformed, command), "{pointer}");
    }
}
