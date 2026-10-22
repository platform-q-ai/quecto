//! #2423 review M2: a message over the 8 MiB frame cap from an extension
//! is dropped unread, so it may be a `tool_result` whose call would
//! otherwise wait out its whole timeout (up to 600 s). The reader fails the
//! client's oldest pending call at once and tells the client why.
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::accept_loop_tests::make_args;
use super::*;
use crate::interface::cli::uds::MAX_FRAME_PAYLOAD_BYTES;

/// The id the accept loop gave the one connected client.
async fn connected_client_id(
    registry: &crate::interface::cli::uds_ext_protocol::ClientToolRegistry,
) -> u64 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let id = registry.lock().unwrap().keys().next().copied();
            match id {
                Some(id) => return id,
                None => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("the client is registered on accept")
}

#[tokio::test]
async fn an_oversized_tool_result_fails_the_pending_call_and_tells_the_extension() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("oversized-result.sock");
    let (args, _bcast, _cmd_tx, _cmd_rx) = make_args(&socket_path, false, None);
    let registry = args.client_tool_registry.clone();
    let handle = spawn_accept_loop(args);
    let client = tokio::net::UnixStream::connect(&socket_path)
        .await
        .expect("connect");
    let (read_half, mut write_half) = client.into_split();
    let client_id = connected_client_id(&registry).await;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    registry
        .lock()
        .unwrap()
        .get_mut(&client_id)
        .unwrap()
        .insert_pending(
            "uds-1".into(),
            "shot".into(),
            reply_tx,
            Duration::from_secs(600),
        );

    let oversized = format!(
        "{{\"type\":\"tool_result\",\"toolCallId\":\"uds-1\",\"content\":\"{}\"}}\n",
        "x".repeat(MAX_FRAME_PAYLOAD_BYTES + 1024)
    );
    let writer = tokio::spawn(async move {
        write_half.write_all(oversized.as_bytes()).await.unwrap();
        write_half
    });

    let failed = tokio::time::timeout(Duration::from_secs(10), reply_rx)
        .await
        .expect("the call is failed long before its timeout")
        .expect("a result is delivered");
    assert!(failed.is_error);
    assert!(
        failed
            .content
            .starts_with("Error: extension result exceeded the 8 MiB frame limit"),
        "{}",
        failed.content
    );

    let mut lines = BufReader::new(read_half).lines();
    let error = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let line = lines.next_line().await.unwrap().expect("connection open");
            let event: serde_json::Value = serde_json::from_str(&line).unwrap();
            if event["command"] == "protocol_error" {
                return event;
            }
        }
    })
    .await
    .expect("the extension is told its message was dropped");
    assert_eq!(error["success"], false);
    let text = error["error"].as_str().unwrap();
    assert!(text.contains("frame cap"), "{text}");
    assert!(text.contains("uds-1"), "names the call it failed: {text}");
    let _ = writer.await;
    handle.abort();
}
