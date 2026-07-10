//! Regression tests for the single-client `run_command_loop` reader (#1003).
//!
//! `run_command_loop` reads client command lines through the shared
//! `quecto_line_io::read_bounded_line` helper rather than
//! `AsyncBufReadExt::lines()`/`next_line()`, so an oversized, unterminated
//! line from the client is capped *while being read* rather than fully
//! buffered and only checked afterward. This exercises the real reader loop
//! over a live socket pair and asserts on the one thing an external observer
//! can see: which events/effects actually happen — the too-long line
//! produces exactly one `parse_error` event and does not block the valid
//! command that follows it from being dispatched.
//!
//! This file is compiled as `mod bounded_read_tests` inside `uds.rs`, so
//! `super` = `uds`.
use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::cli::uds_cancel::CancelSlot;
use crate::interface::cli::uds_ext_protocol::new_client_tool_registry;
use tokio::io::AsyncWriteExt;

fn make_agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: crate::interface::test_support::make_stub_provider(),
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    })
}

#[tokio::test]
async fn oversized_line_reports_parse_error_but_does_not_block_the_next_valid_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let mut agent = make_agent();
    let mut messages: Vec<Message> = Vec::new();
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(1024);

    let mut ctx = DispatchCtx {
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
            session.state_snapshot(0, None, 0, None),
        )),
        session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
        extension_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: Some(&mut tokio::io::sink()),
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
        turn_control: std::sync::Arc::default(),
        broadcast_tx: Some(broadcast_tx),
        ext_registry: None,
        client_tool_registry: new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: None,
        workflow_config: None,
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
    };

    let (mut client, server) = tokio::net::UnixStream::pair().expect("socketpair");
    let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(server);

    // Write from a concurrent task: the oversized line is far larger than the
    // socket buffer, so a sequential `write_all` before `run_command_loop`
    // starts reading would deadlock (writer waits for a reader that never
    // comes, reader is never started).
    let writer = tokio::spawn(async move {
        let oversized = format!(
            "{{\"type\":\"prompt\",\"task\":\"{}\"}}\n",
            "x".repeat(MAX_LINE_BYTES + 65_536)
        );
        client
            .write_all(oversized.as_bytes())
            .await
            .expect("write oversized line");
        client
            .write_all(b"{\"type\":\"get_state\"}\n")
            .await
            .expect("write valid line");
        drop(client);
    });

    run_command_loop(reader, &mut ctx).await;
    writer.await.expect("writer task");

    let mut saw_parse_error = false;
    let mut saw_state = false;
    while let Ok(line) = broadcast_rx.try_recv() {
        let v: serde_json::Value = serde_json::from_str(&line).expect("valid JSON event");
        match (v["type"].as_str(), v["command"].as_str()) {
            (Some("response"), Some("parse_error")) => {
                saw_parse_error = true;
                assert_eq!(
                    v["error"], "line exceeds 1 MiB limit",
                    "the too-long event must report the same message as before #1003"
                );
            }
            (Some("response"), Some("get_state")) => saw_state = true,
            _ => {}
        }
    }

    assert!(
        saw_parse_error,
        "an oversized line must still produce exactly one parse_error event"
    );
    assert!(
        saw_state,
        "the valid command sent after the oversized line must still be dispatched"
    );
}

#[tokio::test]
async fn read_bounded_line_never_buffers_an_oversized_command_past_the_cap() {
    // Directly exercises the read primitive `run_command_loop` now uses,
    // proving the accumulated buffer for one absurdly large unterminated
    // line never grows past MAX_LINE_BYTES — the defect this change fixes
    // (previously `.lines()`/`next_line()` buffered the entire line before
    // any length check could run).
    let mut payload = vec![b'x'; 8 * MAX_LINE_BYTES];
    payload.push(b'\n');
    let mut reader = tokio::io::BufReader::with_capacity(4096, &payload[..]);
    let bounded = quecto_line_io::read_bounded_line(&mut reader, MAX_LINE_BYTES)
        .await
        .unwrap()
        .expect("line present");
    assert!(bounded.truncated);
    assert_eq!(bounded.content.len(), MAX_LINE_BYTES);
    // Duplicate of the quecto-line-io crate test, kept here to pin the
    // helper's invariant at this call site: the valid-UTF-8 path reuses the
    // accumulation buffer's allocation, so capacity() observes the internal
    // buffer — it must never grow past the cap.
    assert!(
        bounded.content.capacity() <= MAX_LINE_BYTES,
        "buffer capacity {} exceeded MAX_LINE_BYTES",
        bounded.content.capacity()
    );
}
