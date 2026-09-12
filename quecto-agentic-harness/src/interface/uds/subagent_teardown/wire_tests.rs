use super::*;

#[test]
fn shutdown_command_parses_with_and_without_correlation_id() {
    let with_id =
        parse_teardown_command(r#"{"type":"shutdown","id":"c-1","reason":"parent_shutdown"}"#)
            .unwrap();
    assert_eq!(
        with_id,
        SubagentTeardownCommand::Shutdown {
            id: Some("c-1".into()),
            reason: "parent_shutdown".into(),
        }
    );
    assert_eq!(with_id.id(), Some("c-1"));
    assert_eq!(with_id.command_name(), "shutdown");
    let without = parse_teardown_command("{\"type\":\"shutdown\",\"reason\":\"x\"}\r\n").unwrap();
    assert_eq!(without.id(), None);
}

#[test]
fn terminate_command_parses_all_fields_and_names_itself() {
    let command = parse_teardown_command(
        r#"{"type":"terminate_delegated_agent","id":"t-9","target_uuid":"B","target_generation":3,"remaining_depth":2}"#,
    )
    .unwrap();
    assert_eq!(
        command,
        SubagentTeardownCommand::TerminateDelegatedAgent {
            id: Some("t-9".into()),
            target_uuid: "B".into(),
            target_generation: 3,
            remaining_depth: 2,
        }
    );
    assert_eq!(command.command_name(), "terminate_delegated_agent");
}

#[test]
fn oversized_claimed_lines_are_refused_with_their_correlation_id() {
    let padding = "x".repeat(TEARDOWN_COMMAND_CAP_BYTES);
    let line = format!(r#"{{"type":"shutdown","id":"big","reason":"{padding}"}}"#);
    assert_eq!(
        parse_teardown_command(&line),
        Err(WireError::Oversized {
            id: Some("big".into()),
            bytes: line.len(),
            cap: TEARDOWN_COMMAND_CAP_BYTES,
        })
    );
    // An oversized line that is not ours is not claimed at all.
    let other = format!(r#"{{"type":"prompt","message":"{padding}"}}"#);
    assert_eq!(
        parse_teardown_command(&other),
        Err(WireError::NotATeardownCommand)
    );
    // Exactly at the cap is still accepted (the trailing newline is not counted).
    let reason =
        "r".repeat(TEARDOWN_COMMAND_CAP_BYTES - r#"{"type":"shutdown","reason":""}"#.len());
    let at_cap = format!("{{\"type\":\"shutdown\",\"reason\":\"{reason}\"}}\n");
    assert_eq!(at_cap.len() - 1, TEARDOWN_COMMAND_CAP_BYTES);
    assert!(parse_teardown_command(&at_cap).is_ok());
}

#[test]
fn only_the_two_teardown_types_are_claimed() {
    for line in [
        r#"{"type":"prompt","message":"hi"}"#,
        r#"{"type":"delete_all_subagents"}"#,
        r#"{"type":"abort"}"#,
        // Not JSON, empty, no object, no type, non-string type: all belong to
        // the ordinary dispatcher and must get no frame from this edge.
        "not json",
        "",
        "[]",
        "null",
        r#"{"reason":"parent_shutdown"}"#,
        r#"{"type":7,"reason":"parent_shutdown"}"#,
        r#"{"type":"Shutdown","reason":"parent_shutdown"}"#,
    ] {
        assert_eq!(
            parse_teardown_command(line),
            Err(WireError::NotATeardownCommand),
            "{line}"
        );
    }
}

#[test]
fn malformed_teardown_commands_are_rejected_affirmatively() {
    let cases = [
        r#"{"type":"shutdown"}"#,
        r#"{"type":"shutdown","reason":7}"#,
        r#"{"type":"shutdown","reason":"parent_shutdown","extra":1}"#,
        r#"{"type":"terminate_delegated_agent","target_uuid":"B"}"#,
        r#"{"type":"terminate_delegated_agent","target_uuid":"B","target_generation":-1,"remaining_depth":1}"#,
        r#"{"type":"terminate_delegated_agent","target_uuid":"B","target_generation":1,"remaining_depth":"1"}"#,
        r#"{"type":"terminate_delegated_agent","target_uuid":"B","target_generation":1,"remaining_depth":1,"ack":"accept"}"#,
    ];
    for line in cases {
        assert!(
            matches!(
                parse_teardown_command(line),
                Err(WireError::Malformed { .. })
            ),
            "{line}"
        );
    }
    // A recognised command with a bad shape keeps its correlation id.
    let Err(WireError::Malformed { id, detail }) =
        parse_teardown_command(r#"{"type":"shutdown","id":"c-9","reason":7}"#)
    else {
        panic!("expected malformed");
    };
    assert_eq!(id.as_deref(), Some("c-9"));
    assert!(detail.contains("invalid type"), "{detail}");
    // A non-string id is itself malformed; it is echoed rendered as text so
    // the peer can still match the rejection.
    assert_eq!(
        parse_teardown_command(r#"{"type":"shutdown","id":5,"reason":"parent_shutdown"}"#),
        Err(WireError::Malformed {
            id: Some("5".into()),
            detail: "id must be a string".into(),
        })
    );
    assert_eq!(
        parse_teardown_command(r#"{"type":"shutdown","id":{"k":1},"reason":"x"}"#),
        Err(WireError::Malformed {
            id: Some("{\"k\":1}".into()),
            detail: "id must be a string".into(),
        })
    );
    // A null id is no id; a duplicated id is a shape violation whose
    // rejection still correlates to the last value given.
    assert_eq!(
        parse_teardown_command(r#"{"type":"shutdown","id":null,"reason":"parent_shutdown"}"#)
            .unwrap()
            .id(),
        None
    );
    let Err(WireError::Malformed { id, detail }) = parse_teardown_command(
        r#"{"type":"shutdown","id":"first","id":"last","reason":"parent_shutdown"}"#,
    ) else {
        panic!("duplicate id is malformed");
    };
    assert_eq!(id.as_deref(), Some("last"));
    assert!(detail.contains("duplicate field"), "{detail}");
}

#[test]
fn wire_errors_render_their_cause() {
    assert_eq!(
        WireError::Oversized {
            id: None,
            bytes: 9,
            cap: 4
        }
        .to_string(),
        "teardown command of 9 bytes exceeds cap 4"
    );
    assert_eq!(
        WireError::NotATeardownCommand.to_string(),
        "not a teardown command"
    );
    assert_eq!(
        WireError::Malformed {
            id: None,
            detail: "eof".into()
        }
        .to_string(),
        "malformed teardown command: eof"
    );
}

#[test]
fn responses_serialize_as_correlated_newline_terminated_frames() {
    let ok = TeardownResponse::ok(
        Some("c-1"),
        SHUTDOWN_COMMAND,
        TeardownResponseData::ShuttingDown {
            reason: "parent_shutdown".into(),
        },
    );
    assert_eq!(
        ok.to_line(),
        "{\"type\":\"response\",\"id\":\"c-1\",\"command\":\"shutdown\",\"success\":true,\"data\":{\"status\":\"shutting_down\",\"reason\":\"parent_shutdown\"}}\n"
    );
    let err = TeardownResponse::err(None, TERMINATE_DELEGATED_AGENT_COMMAND, "nope");
    assert_eq!(
        err.to_line(),
        "{\"type\":\"response\",\"command\":\"terminate_delegated_agent\",\"success\":false,\"error\":\"nope\"}\n"
    );
    let forwarded = TeardownResponse::ok(
        Some("t"),
        TERMINATE_DELEGATED_AGENT_COMMAND,
        TeardownResponseData::Forwarded {
            via_uuid: "A".into(),
            remaining_depth: 2,
        },
    );
    let round_trip: TeardownResponse = serde_json::from_str(forwarded.to_line().trim()).unwrap();
    assert_eq!(round_trip, forwarded);
    let requested = TeardownResponse::ok(
        None,
        TERMINATE_DELEGATED_AGENT_COMMAND,
        TeardownResponseData::ShutdownRequested {
            child_uuid: "B".into(),
        },
    );
    assert!(
        requested
            .to_line()
            .contains("\"status\":\"shutdown_requested\"")
    );
}

#[test]
fn commands_round_trip_through_serialization() {
    for command in [
        SubagentTeardownCommand::Shutdown {
            id: None,
            reason: "operator_request".into(),
        },
        SubagentTeardownCommand::TerminateDelegatedAgent {
            id: Some("x".into()),
            target_uuid: "B".into(),
            target_generation: 1,
            remaining_depth: 1,
        },
    ] {
        let line = serde_json::to_string(&command).unwrap();
        assert_eq!(parse_teardown_command(&line).unwrap(), command);
    }
}
