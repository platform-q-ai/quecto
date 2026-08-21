use super::*;

#[test]
fn test_new_registry_is_empty() {
    let r = new_registry();
    assert!(r.lock().unwrap().is_empty());
}

#[test]
fn test_validate_format_valid() {
    assert!(validate_agent_id_format("abc-123_XYZ").is_ok());
}

#[test]
fn test_validate_format_empty() {
    assert!(validate_agent_id_format("").unwrap_err().contains("1-64"));
}

#[test]
fn test_validate_format_too_long() {
    assert!(
        validate_agent_id_format(&"a".repeat(65))
            .unwrap_err()
            .contains("1-64")
    );
}

#[test]
fn test_validate_format_special_chars() {
    assert!(
        validate_agent_id_format("a/b")
            .unwrap_err()
            .contains("[a-zA-Z0-9_-]")
    );
}

// --- SubagentStatus::to_wire_str ---
#[test]
fn test_status_wire_str_values() {
    assert_eq!(SubagentStatus::Starting.to_wire_str(), "starting");
    assert_eq!(SubagentStatus::Idle.to_wire_str(), "idle");
    assert_eq!(SubagentStatus::Running.to_wire_str(), "running");
    assert_eq!(SubagentStatus::Error.to_wire_str(), "error");
    assert_eq!(SubagentStatus::Exited.to_wire_str(), "exited");
}

// --- SubagentStatus ---

#[test]
fn test_status_display_starting() {
    assert_eq!(format!("{}", SubagentStatus::Starting), "Starting");
}

#[test]
fn test_status_display_idle() {
    assert_eq!(format!("{}", SubagentStatus::Idle), "Idle");
}

#[test]
fn test_status_display_running() {
    assert_eq!(format!("{}", SubagentStatus::Running), "Running");
}

#[test]
fn test_status_display_error() {
    assert_eq!(format!("{}", SubagentStatus::Error), "Error");
}

#[test]
fn test_status_display_exited() {
    assert_eq!(format!("{}", SubagentStatus::Exited), "Exited");
}

#[test]
fn test_status_default_is_starting() {
    assert_eq!(SubagentStatus::default(), SubagentStatus::Starting);
}

#[test]
fn test_all_status_variants_distinct_display() {
    let variants = [
        SubagentStatus::Starting,
        SubagentStatus::Idle,
        SubagentStatus::Running,
        SubagentStatus::Error,
        SubagentStatus::Exited,
    ];
    let displays: Vec<String> = variants.iter().map(|v| format!("{}", v)).collect();
    let unique: std::collections::HashSet<&String> = displays.iter().collect();
    assert_eq!(displays.len(), unique.len());
}

// --- SubagentEntry ---

#[test]
fn test_new_entry_has_starting_status() {
    let entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 42);
    assert_eq!(entry.status, SubagentStatus::Starting);
    assert_eq!(entry.pid, 42);
    assert!(entry.last_tool.is_none());
    assert!(entry.last_error.is_none());
    assert!(entry.monitor_handle.is_none());
}

#[test]
fn test_entry_socket_path() {
    let entry = SubagentEntry::new(PathBuf::from("/run/quecto.sock"), 0);
    assert_eq!(entry.socket_path, PathBuf::from("/run/quecto.sock"));
}

// --- SubagentNotification (#523) ---

#[test]
fn test_completed_message_format() {
    let n = SubagentNotification::Completed {
        agent_id: "researcher".into(),
    };
    let msg = n.to_message();
    assert!(msg.contains("researcher"));
    assert!(msg.contains("ended a turn"));
    assert!(msg.contains("status: idle"));
    assert!(msg.contains("agent_cmd get_messages"));
    assert!(msg.contains("before treating its work as complete"));
    assert!(!msg.contains("finished"));
}

#[test]
fn test_errored_message_format() {
    let n = SubagentNotification::Errored {
        agent_id: "linter".into(),
        error: "rate limit exceeded".into(),
    };
    let msg = n.to_message();
    assert!(msg.contains("linter"));
    assert!(msg.contains("failed"));
    assert!(msg.contains("rate limit exceeded"));
}

#[test]
fn test_exited_message_format() {
    let n = SubagentNotification::Exited {
        agent_id: "formatter".into(),
        reason: None,
    };
    let msg = n.to_message();
    assert!(msg.contains("formatter"));
    assert!(msg.contains("exited"));
}

// --- capped line reader (#795 security review) ---

#[tokio::test]
async fn read_response_capped_reads_lines_then_eof() {
    // Since #1059 the reader sniffs framing from the first byte; legacy NDJSON
    // messages are `{`-opening JSON lines.
    let data = b"{\"n\":1}\n{\"n\":2}\n";
    let mut reader = tokio::io::BufReader::new(&data[..]);
    assert_eq!(
        read_response_capped(&mut reader, 1024)
            .await
            .unwrap()
            .as_deref(),
        Some(r#"{"n":1}"#)
    );
    assert_eq!(
        read_response_capped(&mut reader, 1024)
            .await
            .unwrap()
            .as_deref(),
        Some(r#"{"n":2}"#)
    );
    assert_eq!(read_response_capped(&mut reader, 1024).await.unwrap(), None);
}

#[tokio::test]
async fn read_response_capped_skips_oversized_line_and_reads_the_next() {
    // #1059 review (finding 5a): an over-cap interleaved message must be
    // SKIPPED (its bytes consumed so the stream stays framed) rather than
    // hard-erroring the whole query — mirroring the other four consumers.
    let mut data = format!("{{\"x\":\"{}\"}}\n", "x".repeat(100)).into_bytes();
    data.extend_from_slice(b"{\"n\":2}\n");
    let mut reader = tokio::io::BufReader::new(&data[..]);
    // The oversized first line is skipped; the next valid line is returned.
    assert_eq!(
        read_response_capped(&mut reader, 16)
            .await
            .unwrap()
            .as_deref(),
        Some(r#"{"n":2}"#)
    );
    assert_eq!(read_response_capped(&mut reader, 16).await.unwrap(), None);
}

#[tokio::test]
async fn read_response_capped_reads_framed_replies_then_eof() {
    // #1063 rebase review: since 8322aad3 a same-binary child negotiates
    // framed mode and replies in FRAMES, but every mock child writes legacy
    // NDJSON, so the framed branch of this composed reader was covered only
    // by quecto-line-io unit tests. Pin it here with production-written
    // frames.
    let mut data: Vec<u8> = Vec::new();
    quecto_line_io::write_frame(&mut data, br#"{"n":1}"#, 1024)
        .await
        .unwrap();
    quecto_line_io::write_frame(&mut data, br#"{"n":2}"#, 1024)
        .await
        .unwrap();
    let mut reader = tokio::io::BufReader::new(&data[..]);
    assert_eq!(
        read_response_capped(&mut reader, 1024)
            .await
            .unwrap()
            .as_deref(),
        Some(r#"{"n":1}"#)
    );
    assert_eq!(
        read_response_capped(&mut reader, 1024)
            .await
            .unwrap()
            .as_deref(),
        Some(r#"{"n":2}"#)
    );
    assert_eq!(read_response_capped(&mut reader, 1024).await.unwrap(), None);
}

#[tokio::test]
async fn read_response_capped_skips_oversized_frame_and_reads_the_next() {
    // Framed twin of the legacy oversized-skip test above: an over-cap FRAME
    // must be skipped (declared bytes consumed, stream stays framed), not
    // hard-error the query.
    let mut data: Vec<u8> = Vec::new();
    let big = format!("{{\"x\":\"{}\"}}", "x".repeat(100));
    quecto_line_io::write_frame(&mut data, big.as_bytes(), 1024)
        .await
        .unwrap();
    quecto_line_io::write_frame(&mut data, br#"{"n":2}"#, 1024)
        .await
        .unwrap();
    let mut reader = tokio::io::BufReader::new(&data[..]);
    assert_eq!(
        read_response_capped(&mut reader, 16)
            .await
            .unwrap()
            .as_deref(),
        Some(r#"{"n":2}"#)
    );
    assert_eq!(read_response_capped(&mut reader, 16).await.unwrap(), None);
}

// --- command/response matching (#831) ---

#[test]
fn stamp_request_id_injects_unique_id_into_object() {
    let (out, id) = stamp_request_id(r#"{"type":"get_messages_tail","count":5}"#);
    let id = id.expect("a JSON object command must get an id");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("id").and_then(|x| x.as_str()), Some(id.as_str()));
    // The original fields are preserved.
    assert_eq!(
        v.get("type").and_then(|x| x.as_str()),
        Some("get_messages_tail")
    );
    assert_eq!(v.get("count").and_then(|x| x.as_u64()), Some(5));
    // Successive calls produce distinct ids.
    let (_, id2) = stamp_request_id(r#"{"type":"get_state"}"#);
    assert_ne!(Some(id), id2);
}

#[test]
fn stamp_request_id_overwrites_existing_id() {
    let (out, id) = stamp_request_id(r#"{"type":"get_state","id":"stale"}"#);
    let id = id.unwrap();
    assert_ne!(id, "stale");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("id").and_then(|x| x.as_str()), Some(id.as_str()));
}

#[test]
fn stamp_request_id_none_for_non_object() {
    assert_eq!(stamp_request_id("not json"), ("not json".to_string(), None));
    assert_eq!(stamp_request_id("[1,2,3]"), ("[1,2,3]".to_string(), None));
}

#[tokio::test]
async fn public_command_wrapper_sends_framed_request_and_reads_reply() {
    use tokio::io::BufReader;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("wrapper.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let payload =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .unwrap()
                .unwrap();
        let req: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(req["type"], "get_state");
        let id = req["id"].as_str().unwrap();
        let reply = format!(
            r#"{{"type":"response","id":"{id}","command":"get_state","data":{{"status":"ok"}}}}"#
        );
        quecto_line_io::write_frame(
            &mut write_half,
            reply.as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    });

    let reply = send_subagent_uds_command(&sock, r#"{"type":"get_state"}"#)
        .await
        .unwrap();
    assert!(reply.contains(r#""status":"ok""#));
    server.await.unwrap();
}

#[tokio::test]
async fn command_reader_skips_connect_time_snapshot_and_returns_matching_reply() {
    // Reproduce #831: a BUSY child pushes an unsolicited connect-time
    // `get_messages` SNAPSHOT as the FIRST line, then the real reply. The
    // snapshot carries no `id`, while the real reply echoes the request id the
    // helper stamped — so the reader must return the latter (latest turns), not
    // the snapshot (the child's first message only). This also proves the fix
    // generalises to a `get_messages` request: the snapshot shares its command
    // string, yet id-correlation still skips it.
    use tokio::io::{AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("busy.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        // Unsolicited connect-time snapshot (the child's FIRST message), no id.
        write_half
            .write_all(b"{\"type\":\"response\",\"command\":\"get_messages\",\"data\":[{\"content\":\"FIRST MESSAGE ONLY\"}]}\n")
            .await
            .unwrap();
        // A same-binary parent must write a length-prefixed frame regardless
        // of the compatibility reader's per-message sniffing mode.
        let mut reader = BufReader::new(read_half);
        let payload =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .expect("parent command should be framed")
                .expect("parent should send a command");
        let req: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        let id = req.get("id").and_then(|v| v.as_str()).unwrap();
        let reply = format!(
            "{{\"type\":\"response\",\"id\":\"{id}\",\"command\":\"get_messages_tail\",\"data\":[{{\"content\":\"LATEST TURNS\"}}]}}\n"
        );
        write_half.write_all(reply.as_bytes()).await.unwrap();
        // Keep the connection alive briefly so the reader can consume both lines.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    let cmd = r#"{"type":"get_messages_tail","count":5}"#;
    let reply =
        send_subagent_uds_command_with_timeout(&sock, cmd, std::time::Duration::from_secs(3))
            .await
            .expect("reader should return a response");

    assert!(
        reply.contains("get_messages_tail") && reply.contains("LATEST TURNS"),
        "expected the get_messages_tail reply, got: {reply}"
    );
    assert!(
        !reply.contains("FIRST MESSAGE ONLY"),
        "must not return the connect-time snapshot, got: {reply}"
    );
    server.await.unwrap();
}

// --- notification channel ---

#[tokio::test]
async fn test_notification_channel_bounded() {
    let (tx, _rx) = new_notification_channel();
    for i in 0..NOTIFICATION_CHANNEL_CAPACITY {
        let n = SubagentNotification::Completed {
            agent_id: format!("bot-{}", i),
        };
        assert!(
            tx.try_send(SequencedSubagentNotification::new(i as u64 + 1, n))
                .is_ok()
        );
    }
}

#[tokio::test]
async fn test_notification_drain() {
    let (tx, mut rx) = new_notification_channel();
    for i in 0..3 {
        let _ = tx
            .send(SequencedSubagentNotification::new(
                i as u64 + 1,
                SubagentNotification::Exited {
                    agent_id: format!("bot-{}", i),
                    reason: None,
                },
            ))
            .await;
    }
    drop(tx);
    let mut count = 0;
    while rx.recv().await.is_some() {
        count += 1;
    }
    assert_eq!(count, 3);
}

// Cascade-remove tests moved to `subagent_cascade_tests.rs` alongside the
// extracted `subagent_cascade` module (#831).

#[test]
fn snapshot_response_is_valid_for_uncounted_get_messages_and_get_state_only() {
    let messages_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_messages",
        "data": { "messages": [] }
    });
    assert!(subagent_snapshot::response_is_valid_answer(
        &messages_snapshot,
        r#"{"type":"get_messages"}"#
    ));
    let correlated_messages = serde_json::json!({
        "type": "response",
        "id": "other-request",
        "command": "get_messages",
        "data": { "messages": [] }
    });
    assert!(!subagent_snapshot::response_is_valid_answer(
        &correlated_messages,
        r#"{"type":"get_messages"}"#
    ));
    // #842: a counted get_messages is now served from the snapshot too (the
    // parent applies `count` locally), so the id-less snapshot is a valid answer.
    assert!(subagent_snapshot::response_is_valid_answer(
        &messages_snapshot,
        r#"{"type":"get_messages","count":1}"#
    ));
    // ...but an agent_id (a DIFFERENT target) must still reject the snapshot.
    assert!(!subagent_snapshot::response_is_valid_answer(
        &messages_snapshot,
        r#"{"type":"get_messages","count":1,"agent_id":"grandchild"}"#
    ));
    // #1061: a `before` cursor must never be answered by the connect-time
    // snapshot — it is always the NEWEST page, so it would echo the caller's
    // own cursor back and the paging loop would spin without advancing.
    assert!(!subagent_snapshot::response_is_valid_answer(
        &messages_snapshot,
        r#"{"type":"get_messages","before":"some-cursor"}"#
    ));

    let state_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "data": {
            "state": "runningTool",
            "effort": null,
            "model": "mock",
            "progress": { "state": "advancing", "reason": "tool activity" },
            "generation": 7
        }
    });
    assert!(subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_state"}"#
    ));
    assert!(subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_state","since":6}"#
    ));
    assert!(subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_state","since":7}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_state","since":8}"#
    ));
    let unchanged_line = subagent_snapshot::finalize_snapshot_answer(
        state_snapshot.to_string(),
        state_snapshot.clone(),
        r#"{"type":"get_state","since":7}"#,
    );
    let unchanged_json: serde_json::Value = serde_json::from_str(&unchanged_line).unwrap();
    assert_eq!(
        unchanged_json["data"],
        serde_json::json!({ "unchanged": true, "generation": 7 })
    );
    let unchanged_state_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "data": { "unchanged": true, "generation": 7 }
    });
    assert!(subagent_snapshot::response_is_valid_answer(
        &unchanged_state_snapshot,
        r#"{"type":"get_state","since":7}"#
    ));
    let mut bulky_state_snapshot = state_snapshot.clone();
    bulky_state_snapshot["data"]["effortLevels"] = serde_json::json!(["low", "medium"]);
    bulky_state_snapshot["data"]["workflow"] = serde_json::json!({
        "activeTemplate":{"id":"bugfix"},
        "currentStep":{"index":1,"key":"red","label":"RED","phase":"RED","done":false},
        "available_templates":[{"id":"bugfix"}]
    });
    assert!(!subagent_snapshot::response_is_valid_answer(
        &bulky_state_snapshot,
        r#"{"type":"get_state"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &serde_json::json!({"type":"response","command":"get_state","data":{"isStreaming":true,"messageCount":2,"snapshot":true}}),
        r#"{"type":"get_state"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &serde_json::json!({"type":"response","command":"get_state","data":{"state":"idle","generation":7}}),
        r#"{"type":"get_state"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_session_stats"}"#
    ));
    // #874: a get_subagents snapshot is a valid answer for a get_subagents
    // request (the registry is independent of the dispatch loop, so a busy child
    // can serve it off the turn).
    let subagents_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "data": { "subagents": [] }
    });
    assert!(subagent_snapshot::response_is_valid_answer(
        &subagents_snapshot,
        r#"{"type":"get_subagents"}"#
    ));
    let correlated_subagents = serde_json::json!({
        "type": "response",
        "id": "other-request",
        "command": "get_subagents",
        "data": { "subagents": [] }
    });
    assert!(!subagent_snapshot::response_is_valid_answer(
        &correlated_subagents,
        r#"{"type":"get_subagents"}"#
    ));
    // #880: get_session_stats and get_tool_catalogue snapshots are valid answers
    // for their own request commands (pure reads, independent of the blocked
    // dispatch loop), but still never answer a different command.
    let stats_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_session_stats",
        "data": { "sessionKey": "cli:test", "userMessages": 1, "snapshot": true }
    });
    assert!(subagent_snapshot::response_is_valid_answer(
        &stats_snapshot,
        r#"{"type":"get_session_stats"}"#
    ));
    let malformed_stats_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_session_stats",
        "data": { "sessionKey": "cli:test", "userMessages": 1 }
    });
    assert!(!subagent_snapshot::response_is_valid_answer(
        &malformed_stats_snapshot,
        r#"{"type":"get_session_stats"}"#
    ));
    let tool_catalogue_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_tool_catalogue",
        "data": { "tools": [], "snapshot": true }
    });
    assert!(subagent_snapshot::response_is_valid_answer(
        &tool_catalogue_snapshot,
        r#"{"type":"get_tool_catalogue"}"#
    ));
    let malformed_tool_catalogue_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_tool_catalogue",
        "data": { "tools": [] }
    });
    assert!(!subagent_snapshot::response_is_valid_answer(
        &malformed_tool_catalogue_snapshot,
        r#"{"type":"get_tool_catalogue"}"#
    ));
    // A get_subagents snapshot must NOT answer a different command (#835).
    assert!(!subagent_snapshot::response_is_valid_answer(
        &subagents_snapshot,
        r#"{"type":"get_session_stats"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &stats_snapshot,
        r#"{"type":"get_subagents"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &tool_catalogue_snapshot,
        r#"{"type":"get_session_stats"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_subagents"}"#
    ));
    // A malformed get_subagents snapshot (missing the subagents array) is rejected.
    let malformed_subagents = serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "data": { }
    });
    assert!(!subagent_snapshot::response_is_valid_answer(
        &malformed_subagents,
        r#"{"type":"get_subagents"}"#
    ));
}

#[test]
fn snapshot_get_subagents_rejects_agent_id_targeted_request() {
    // A get_subagents request that targets a NESTED agent (via agent_id) must
    // NOT be answered by THIS child's own registry snapshot — it must round-trip
    // to the named descendant (#835 id-correlation, #874 extension).
    let subagents_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "data": { "subagents": [] }
    });
    assert!(!subagent_snapshot::response_is_valid_answer(
        &subagents_snapshot,
        r#"{"type":"get_subagents","agent_id":"grandchild"}"#
    ));
}

#[test]
fn finalize_snapshot_answer_tails_to_count() {
    let snapshot = serde_json::json!({
        "type": "response",
        "command": "get_messages",
        "data": { "messages": [
            {"role": "user", "content": "m1"},
            {"role": "assistant", "content": "m2"},
            {"role": "user", "content": "m3"},
        ], "snapshot": true }
    });
    let raw = snapshot.to_string();
    // count=2 keeps the last two messages, preserving the snapshot marker.
    let line = subagent_snapshot::finalize_snapshot_answer(
        raw.clone(),
        snapshot.clone(),
        r#"{"type":"get_messages","count":2}"#,
    );
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    let msgs = v.pointer("/data/messages").unwrap().as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["content"], "m2");
    assert_eq!(msgs[1]["content"], "m3");
    assert_eq!(v.pointer("/data/snapshot"), Some(&serde_json::json!(true)));

    // No count => the original line is returned VERBATIM (no re-encode).
    let line = subagent_snapshot::finalize_snapshot_answer(
        raw.clone(),
        snapshot.clone(),
        r#"{"type":"get_messages"}"#,
    );
    assert_eq!(line, raw, "uncounted request returns the line unchanged");

    // count larger than history => all messages, no panic.
    let line = subagent_snapshot::finalize_snapshot_answer(
        raw,
        snapshot,
        r#"{"type":"get_messages","count":99}"#,
    );
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        v.pointer("/data/messages")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn finalize_snapshot_answer_count_slice_repoints_page_cursor() {
    // #1061 review follow-up: slicing to last-N drops older snapshot messages,
    // so the page metadata must be recomputed — `before` names the oldest
    // message still INCLUDED and older history now definitely exists. A stale
    // cursor would make a caller skip the dropped span.
    let snapshot = serde_json::json!({
        "type": "response",
        "command": "get_messages",
        "data": { "messages": [
            {"id": "id-1", "role": "user", "content": "m1"},
            {"id": "id-2", "role": "assistant", "content": "m2"},
            {"id": "id-3", "role": "user", "content": "m3"},
        ], "snapshot": true, "hasMoreBefore": false }
    });
    let line = subagent_snapshot::finalize_snapshot_answer(
        snapshot.to_string(),
        snapshot.clone(),
        r#"{"type":"get_messages","count":2}"#,
    );
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        v.pointer("/data/before"),
        Some(&serde_json::json!("id-2")),
        "cursor must point at the oldest INCLUDED message: {v}"
    );
    assert_eq!(
        v.pointer("/data/hasMoreBefore"),
        Some(&serde_json::json!(true))
    );

    // An unsliced answer keeps the snapshot's own metadata untouched.
    let line = subagent_snapshot::finalize_snapshot_answer(
        snapshot.to_string(),
        snapshot,
        r#"{"type":"get_messages","count":99}"#,
    );
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        v.pointer("/data/hasMoreBefore"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(v.pointer("/data/before"), None);
}

#[test]
fn snapshot_response_rejects_invalid_or_mismatched_commands() {
    let state_snapshot = serde_json::json!({"type":"response","command":"get_state","data":{}});
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        "not-json"
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"count":1}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_state","agent_id":"child"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_messages"}"#
    ));
}
