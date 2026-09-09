use super::*;
use std::time::Duration;

#[tokio::test]
async fn public_get_state_uses_inspection_deadline_without_accept_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("stalled.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("worker".into(), SubagentEntry::new(socket, 0));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = tokio::io::BufReader::new(stream);
        let request =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .unwrap()
                .unwrap();
        let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
        assert_eq!(request["type"], "get_state");
        assert!(request.get("ack").is_none());
        tokio::time::sleep(Duration::from_secs(20)).await;
    });
    let result = tokio::time::timeout(
        Duration::from_secs(7),
        AgentCmdTool::new(registry).execute(r#"{"agent_id":"worker","command":"get_state"}"#),
    )
    .await;
    server.abort();
    let result = result
        .expect("public inspection must not use the 300s long-operation deadline")
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("timed out"), "{}", result.content);
}
