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
