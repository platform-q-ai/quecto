use super::*;
use std::path::PathBuf;

fn test_entry() -> SubagentEntry {
    SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0)
}

// --- bounded read at read time (#1003) ---
//
// `monitor_loop` reads child event lines through the shared
// `quecto_line_io::read_bounded_line` helper rather than
// `AsyncBufReadExt::lines()`/`next_line()`, so an oversized, unterminated
// line from a misbehaving child is capped *while being read* instead of
// being fully buffered and only checked afterward. These tests exercise
// that behaviour end-to-end through `spawn_monitor_task` in terms of the
// one thing an external observer can see: whether events keep flowing.

#[tokio::test]
async fn monitor_loop_drops_oversized_line_but_keeps_processing_later_events() {
    use tokio::io::AsyncWriteExt;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("child.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let (btx, mut brx) = tokio::sync::broadcast::channel::<String>(8);
    let handle = spawn_monitor_task(
        "child".to_string(),
        sock.clone(),
        registry.clone(),
        None,
        MonitorContext {
            broadcast_tx: Some(btx),
            parent_id: Some("root".to_string()),
            container_registry: None,
        },
    );
    let (mut stream, _) = listener.accept().await.unwrap();

    // One giant unterminated-then-terminated line, well over MAX_EVENT_PAYLOAD_BYTES,
    // followed by a normal, valid workflow_state event.
    let oversized_payload = "x".repeat(super::MAX_EVENT_PAYLOAD_BYTES + 65_536);
    stream
        .write_all(format!("{{\"type\":\"noop\",\"pad\":\"{oversized_payload}\"}}\n").as_bytes())
        .await
        .unwrap();
    stream
        .write_all(b"{\"type\":\"workflow_state\",\"mode\":\"active\",\"progress\":{\"done\":1,\"total\":2}}\n")
        .await
        .unwrap();

    let line = tokio::time::timeout(std::time::Duration::from_secs(3), brx.recv())
        .await
        .expect("monitor should keep forwarding events after an oversized line within 3s")
        .expect("broadcast line");
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["agent_id"], "child");
    assert_eq!(
        v["mode"], "active",
        "the oversized line must not be delivered as a parsed event, only the valid one that follows it"
    );
    handle.abort();
}

#[tokio::test]
async fn monitor_loop_never_buffers_an_oversized_line_past_the_cap() {
    // Directly exercises the read primitive `monitor_loop` now uses, proving
    // the accumulated buffer for one absurdly large unterminated line never
    // grows past MAX_EVENT_PAYLOAD_BYTES — the defect this change fixes (previously
    // `.lines()`/`next_line()` buffered the entire line before any length
    // check could run).
    let mut payload = vec![b'x'; 8 * super::MAX_EVENT_PAYLOAD_BYTES];
    payload.push(b'\n');
    let mut reader = tokio::io::BufReader::with_capacity(4096, &payload[..]);
    let bounded = quecto_line_io::read_bounded_line(&mut reader, super::MAX_EVENT_PAYLOAD_BYTES)
        .await
        .unwrap()
        .expect("line present");
    assert!(bounded.truncated);
    assert_eq!(bounded.content.len(), super::MAX_EVENT_PAYLOAD_BYTES);
    // Duplicate of the quecto-line-io crate test, kept here to pin the
    // helper's invariant at this call site: the valid-UTF-8 path reuses the
    // accumulation buffer's allocation, so capacity() observes the internal
    // buffer — it must never grow past the cap.
    assert!(
        bounded.content.capacity() <= super::MAX_EVENT_PAYLOAD_BYTES,
        "buffer capacity {} exceeded MAX_EVENT_PAYLOAD_BYTES",
        bounded.content.capacity()
    );
}
