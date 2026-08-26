//! Coverage-focused tests for registry lookup error paths, poisoned-lock
//! recovery, and the UDS command sender's failure/skip branches.

use super::super::*;
use crate::domain::ids::AgentUuid;
use std::panic::AssertUnwindSafe;

fn live_entry(uuid: &str, name: &str, sock: &str, pid: u32) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::from(uuid.to_string()),
        name.to_string(),
        sock.into(),
        pid,
    );
    entry.status = SubagentStatus::Idle;
    entry
}

#[test]
fn lookup_subagent_socket_reports_unknown_display_name_as_not_found() {
    let reg = new_registry();
    reg.lock().unwrap().insert(
        "uuid-a".to_string(),
        live_entry("uuid-a", "worker", "/tmp/a.sock", 1),
    );

    let err = lookup_subagent_socket(&reg, "ghost").unwrap_err();
    assert_eq!(err, "no live subagent named 'ghost' (not found)");
}

#[test]
fn lookup_subagent_socket_reports_duplicate_display_names_as_ambiguous() {
    let reg = new_registry();
    {
        let mut entries = reg.lock().unwrap();
        entries.insert(
            "uuid-a".to_string(),
            live_entry("uuid-a", "worker", "/tmp/a.sock", 1),
        );
        entries.insert(
            "uuid-b".to_string(),
            live_entry("uuid-b", "worker", "/tmp/b.sock", 2),
        );
    }
    let err = lookup_subagent_socket(&reg, "worker").unwrap_err();
    assert_eq!(err, "duplicate live subagent display label 'worker'");
}

#[test]
fn has_active_descendant_for_agent_recovers_from_poisoned_registry() {
    let reg = new_registry();
    {
        let mut entries = reg.lock().unwrap();
        entries.insert(
            "uuid-p".to_string(),
            live_entry("uuid-p", "parent", "/tmp/p.sock", 1),
        );
        let mut child = live_entry("uuid-c", "child", "/tmp/c.sock", 2);
        child.parent_id = Some("uuid-p".to_string());
        child.status = SubagentStatus::Running;
        entries.insert("uuid-c".to_string(), child);
    }
    let poison = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _guard = reg.lock().unwrap();
        panic!("poison registry for coverage test");
    }));
    assert!(poison.is_err());
    assert!(reg.is_poisoned(), "registry mutex must be poisoned");

    let registry = Some(reg);
    assert!(
        has_active_descendant_for_agent(&registry, "uuid-p"),
        "running child must be detected despite the poisoned lock"
    );
    assert!(!has_active_descendant_for_agent(&registry, "uuid-c"));
}

#[tokio::test]
async fn send_subagent_uds_command_fails_fast_when_socket_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("missing.sock");
    let err = send_subagent_uds_command_with_timeout(
        &sock,
        r#"{"type":"get_state"}"#,
        std::time::Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("connect to subagent at"), "{msg}");
    assert!(msg.contains("missing.sock"), "{msg}");
}

#[tokio::test]
async fn send_subagent_uds_command_times_out_when_child_never_replies() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("silent.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        // Hold the connection open without ever answering.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        drop(stream);
    });

    let err = send_subagent_uds_command_with_timeout(
        &sock,
        r#"{"type":"get_state"}"#,
        std::time::Duration::from_millis(200),
    )
    .await
    .unwrap_err();
    server.abort();
    let msg = format!("{err}");
    assert!(msg.contains("subagent response timed out (0s)"), "{msg}");
}

#[tokio::test]
async fn send_subagent_uds_command_skips_invalid_utf8_then_accepts_matching_reply() {
    use tokio::io::BufReader;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("utf8.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let payload =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .expect("command frame")
                .expect("command sent");
        let command: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        let id = command["id"].as_str().unwrap().to_string();

        // First reply: invalid UTF-8, exercising the lossy-decode branch;
        // the sender must skip it because it is not a matching response.
        quecto_line_io::write_frame(
            &mut write_half,
            &[0xFF, 0xFE, b'{', b'}'],
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();

        let reply = serde_json::json!({"type": "response", "id": id, "content": "pong"});
        quecto_line_io::write_frame(
            &mut write_half,
            reply.to_string().as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
        // Keep the write half open until the client is done.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    });

    let reply = send_subagent_uds_command_with_timeout(
        &sock,
        r#"{"type":"ping"}"#,
        std::time::Duration::from_secs(10),
    )
    .await
    .expect("id-matched reply must be accepted after the garbage frame");
    server.abort();
    let json: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(json["content"], "pong");
}
