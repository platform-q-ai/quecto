use super::*;

fn ack_json(line: &str) -> serde_json::Value {
    serde_json::from_str(line.trim_end()).expect("ack line is valid JSON")
}

#[test]
fn flagged_prompt_acks_with_id_and_forwards_follow_up() {
    let line = r#"{"type":"prompt","message":"do work","ack":"accept","id":"req-1"}"#;
    let got = intercept_control_forward(line).expect("flagged prompt is intercepted");

    let ack = ack_json(&got.ack_line);
    assert_eq!(ack["type"], "response");
    assert_eq!(ack["id"], "req-1", "ack must echo the request id (#835)");
    assert_eq!(ack["command"], "prompt", "ack echoes the original command");
    assert_eq!(ack["success"], true);
    assert!(
        got.ack_line.ends_with('\n'),
        "ack line is newline-terminated"
    );

    // A fresh prompt is forwarded as a follow_up so a busy child QUEUES it
    // rather than rejecting it for a missing streamingBehavior.
    let fwd: serde_json::Value =
        serde_json::from_str(&got.forward_line.expect("prompt forwards work")).unwrap();
    assert_eq!(fwd["type"], "follow_up");
    assert_eq!(fwd["message"], "do work");
    assert!(
        fwd.get("ack").is_none(),
        "marker is stripped before dispatch"
    );
    assert!(fwd.get("id").is_none(), "id is stripped before dispatch");
}

#[test]
fn flagged_follow_up_forwards_follow_up() {
    let line = r#"{"type":"follow_up","message":"next","ack":"accept","id":"r2"}"#;
    let got = intercept_control_forward(line).expect("flagged follow_up is intercepted");
    assert_eq!(ack_json(&got.ack_line)["command"], "follow_up");
    let fwd: serde_json::Value = serde_json::from_str(&got.forward_line.unwrap()).unwrap();
    assert_eq!(fwd["type"], "follow_up");
    assert_eq!(fwd["message"], "next");
}

#[test]
fn flagged_steer_forwards_prompt_with_steer_streaming_behavior() {
    for line in [
        r#"{"type":"steer","message":"turn left","ack":"accept","id":"r3"}"#,
        r#"{"type":"prompt","message":"turn left","streamingBehavior":"steer","ack":"accept","id":"r3"}"#,
    ] {
        let got = intercept_control_forward(line).expect("flagged steer is intercepted");
        assert_eq!(ack_json(&got.ack_line)["command"], "steer");
        let fwd: serde_json::Value = serde_json::from_str(&got.forward_line.unwrap()).unwrap();
        assert_eq!(fwd["type"], "prompt");
        assert_eq!(fwd["streamingBehavior"], "steer");
        assert_eq!(fwd["message"], "turn left");
    }
}

#[test]
fn flagged_abort_acks_with_no_forward() {
    let line = r#"{"type":"abort","ack":"accept","id":"r4"}"#;
    let got = intercept_control_forward(line).expect("flagged abort is intercepted");
    assert_eq!(ack_json(&got.ack_line)["command"], "abort");
    assert!(
        got.forward_line.is_none(),
        "abort only needs the cancel the reader already fired"
    );
}

#[test]
fn ack_falls_back_to_no_id_when_unstamped() {
    let line = r#"{"type":"prompt","message":"hi","ack":"accept"}"#;
    let got = intercept_control_forward(line).expect("intercepted");
    let ack = ack_json(&got.ack_line);
    assert!(ack.get("id").is_none(), "no id field when none was sent");
}

#[test]
fn unflagged_control_commands_are_not_intercepted() {
    // Interactive TUI/CLI prompts (no marker) must dispatch normally.
    for line in [
        r#"{"type":"prompt","message":"hello"}"#,
        r#"{"type":"steer","message":"hello"}"#,
        r#"{"type":"follow_up","message":"hello"}"#,
        r#"{"type":"abort"}"#,
    ] {
        assert!(
            intercept_control_forward(line).is_none(),
            "unflagged line must not be intercepted: {line}"
        );
    }
}

#[test]
fn flagged_prompt_without_message_is_not_intercepted() {
    let line = r#"{"type":"prompt","ack":"accept","id":"r5"}"#;
    assert!(intercept_control_forward(line).is_none());
}

#[test]
fn flagged_remaining_queueable_commands_ack_and_forward_without_marker() {
    for (line, command) in [
        (
            r#"{"type":"set_model","model":"anthropic/claude-sonnet-4-6","ack":"accept","id":"m1"}"#,
            "set_model",
        ),
        (
            r#"{"type":"clear_history","ack":"accept","id":"c1"}"#,
            "clear_history",
        ),
    ] {
        let got = intercept_control_forward(line).expect("queueable command is intercepted");
        assert_eq!(ack_json(&got.ack_line)["command"], command);
        let fwd: serde_json::Value = serde_json::from_str(&got.forward_line.unwrap()).unwrap();
        assert_eq!(fwd["type"], command);
        assert!(fwd.get("ack").is_none(), "marker is stripped");
        assert!(fwd.get("id").is_none(), "id is stripped before dispatch");
    }
}

#[test]
fn flagged_non_control_command_is_not_intercepted() {
    let line = r#"{"type":"get_state","ack":"accept","id":"r6"}"#;
    assert!(intercept_control_forward(line).is_none());
}

#[test]
fn garbage_and_marker_in_string_body_do_not_intercept() {
    assert!(intercept_control_forward("not json").is_none());
    assert!(intercept_control_forward("{}").is_none());
    // The cheap substring gate may match, but the structured check rejects a
    // prompt whose body merely mentions the marker.
    let line = r#"{"type":"prompt","message":"the word accept appears here"}"#;
    assert!(intercept_control_forward(line).is_none());
}
