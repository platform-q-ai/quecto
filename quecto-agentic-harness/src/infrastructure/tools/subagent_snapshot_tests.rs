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

    // #2210 review: a member a newer child adds, at any boundary, is
    // accepted and dropped from what is relayed; a name that is no member's
    // is still refused.
    for (pointer, value) in [
        ("/data/progress/extra", serde_json::json!(true)),
        ("/data/workflow/extra", serde_json::json!(true)),
        (
            "/data/workflow/activeTemplate/name",
            serde_json::json!("Bugfix"),
        ),
        ("/data/workflow/currentStep/extra", serde_json::json!(true)),
    ] {
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        for (name, accepted) in [(key, true), ("Bad_Name", false)] {
            let mut added = base.clone();
            added
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(name.into(), value.clone());
            assert_eq!(
                response_is_valid_answer(&added, command),
                accepted,
                "{parent}/{name}"
            );
            if accepted {
                let relayed: serde_json::Value = serde_json::from_str(&finalize_snapshot_answer(
                    added.to_string(),
                    added.clone(),
                    command,
                ))
                .unwrap();
                let plain: serde_json::Value = serde_json::from_str(&finalize_snapshot_answer(
                    base.to_string(),
                    base.clone(),
                    command,
                ))
                .unwrap();
                assert_eq!(relayed, plain, "{pointer} is not relayed");
            }
        }
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
    // #2210 review: a member a newer child adds, at any boundary, is
    // accepted (forward compatibility) and never relayed.
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
        assert!(response_is_valid_answer(&malformed, command), "{pointer}");
        let relayed = finalize_snapshot_answer(malformed.to_string(), malformed, command);
        assert!(!relayed.contains("\"extra\""), "{pointer}: {relayed}");
    }
}

/// #2210: a busy child's snapshot carrying its model turn in flight is a
/// valid `get_state` answer; the turn's closed shape still refuses unknown
/// or malformed members.
#[test]
fn a_busy_snapshot_with_its_model_turn_is_a_valid_get_state_answer() {
    let command = r#"{"type":"get_state"}"#;
    let base = serde_json::json!({
        "type": "response", "command": "get_state",
        "data": {
            "state": "thinking", "effort": "low", "model": "mock",
            "sessionKey": "cli:child",
            "progress": { "state": "active", "reason": "thinking" },
            "generation": 3,
            "modelTurn": {
                "elapsedMs": 504_000,
                "outputCapBytes": 1_024_000,
                "attempt": {
                    "number": 1, "elapsedMs": 503_000, "events": 9_000,
                    "outputBytes": 640_000, "sinceLastEventMs": 40,
                    "firstTokenMs": 2_100
                }
            }
        }
    });
    assert!(response_is_valid_answer(&base, command));
    let mut waiting = base.clone();
    waiting["data"]["modelTurn"] = serde_json::json!({"elapsedMs": 12});
    assert!(response_is_valid_answer(&waiting, command));
    for pointer in ["/data/modelTurn/extra", "/data/modelTurn/attempt/extra"] {
        let mut added = base.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        added
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(key.into(), serde_json::json!(1));
        assert!(response_is_valid_answer(&added, command), "{pointer}");
    }
    for (pointer, value) in [
        ("/data/modelTurn/Extra", serde_json::json!(true)),
        ("/data/modelTurn/attempt/bad_name", serde_json::json!(1)),
        ("/data/modelTurn/elapsedMs", serde_json::json!("soon")),
        (
            "/data/modelTurn/attempt/sinceLastEventMs",
            serde_json::Value::Null,
        ),
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
            "malformed model turn {pointer} must be rejected"
        );
    }
}

/// #2210 review: a `since` cursor never hides a model turn in flight — the
/// busy child's snapshot is passed through in full, while one without a
/// model turn still collapses to the unchanged marker.
#[test]
fn a_since_cursor_never_hides_a_model_turn_in_flight() {
    let mut state = production_state();
    state.execution = Some(Default::default());
    state.execution.as_mut().unwrap().model_turn =
        Some(serde_json::from_value(serde_json::json!({"elapsedMs": 480_000})).unwrap());
    let response = production_response(&state);
    for since in [6, 7, 8] {
        let command = serde_json::json!({"type":"get_state","since":since}).to_string();
        assert!(response_is_valid_answer(&response, &command), "{response}");
        let answer: serde_json::Value = serde_json::from_str(&finalize_snapshot_answer(
            response.to_string(),
            response.clone(),
            &command,
        ))
        .unwrap();
        assert_eq!(
            answer["data"]["modelTurn"]["elapsedMs"], 480_000,
            "since={since}"
        );
        // A current cursor gets the small marker carrying the turn, never
        // the whole projection; a stale one gets the projection.
        match since >= 7 {
            true => assert_eq!(
                answer["data"],
                serde_json::json!({"unchanged": true, "generation": since,
                                   "modelTurn": {"elapsedMs": 480_000}})
            ),
            false => assert_eq!(answer["data"]["generation"], 7),
        }
    }
    // A child's own marker carrying the turn is accepted and relayed at the
    // caller's cursor.
    let marker = serde_json::json!({"type": "response", "command": "get_state",
        "data": {"unchanged": true, "generation": 7, "modelTurn": {"elapsedMs": 5}}});
    for (since, accepted) in [(6, false), (7, true), (8, true)] {
        let command = serde_json::json!({"type":"get_state","since":since}).to_string();
        assert_eq!(
            response_is_valid_answer(&marker, &command),
            accepted,
            "since={since}"
        );
        if accepted {
            let answer: serde_json::Value = serde_json::from_str(&finalize_snapshot_answer(
                marker.to_string(),
                marker.clone(),
                &command,
            ))
            .unwrap();
            assert_eq!(
                answer["data"],
                serde_json::json!({"unchanged": true, "generation": since,
                                   "modelTurn": {"elapsedMs": 5}})
            );
        }
    }
    let idle = production_response(&production_state());
    let command = r#"{"type":"get_state","since":7}"#;
    let answer: serde_json::Value = serde_json::from_str(&finalize_snapshot_answer(
        idle.to_string(),
        idle.clone(),
        command,
    ))
    .unwrap();
    assert_eq!(
        answer["data"],
        serde_json::json!({"unchanged": true, "generation": 7})
    );
}
