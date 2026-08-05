//! Scenario event builders and frame-normalization helper for the headless
//! render harness ([`super::TuiHarness`]). Split out of `tui_harness.rs` to keep
//! that file within the repo's per-file line budget (#805).

use super::SEQ;
use crate::protocol::client::{Event, SubagentInfoEvent, SubagentWorkflow};
use std::sync::atomic::Ordering;
use tokio::io::BufReader;
use tokio::sync::mpsc;

/// Collapse spinner frames and digit runs so repeated renders compare equal
/// regardless of animation phase / counters. Visible to the parent module's
/// flash-detection logic.
pub(super) fn normalize(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut last_was_digit = false;
    for c in line.chars() {
        if ('\u{2800}'..='\u{28ff}').contains(&c) {
            continue; // braille spinner frame
        }
        if c.is_ascii_digit() {
            if !last_was_digit {
                out.push('#');
            }
            last_was_digit = true;
            continue;
        }
        last_was_digit = false;
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Scenario event builders ───────────────────────────────────────────

/// A `SubagentInfoEvent`, optionally carrying a workflow snapshot `(mode, done, total)`.
pub fn subagent(id: &str, status: &str, wf: Option<(&str, u32, u32)>) -> SubagentInfoEvent {
    subagent_with_socket(id, status, wf, None)
}

/// Bind a real, drained Unix socket for a sub-agent and return its path. The
/// listener task accepts one connection and drains its lines, so a TUI
/// ledger-sync feed connection to this path succeeds and the per-child command
/// channel stays live (its receiver is NOT dropped) — letting routing tests
/// exercise the real `try_send` delivery path rather than the older-kernel
/// `None` case.
pub fn spawn_subagent_socket(id: &str) -> std::path::PathBuf {
    let (path, mut cmd_rx) = spawn_subagent_socket_with_commands(id);
    // Keep the receiver alive and drain commands so every accepted connection
    // exercises the real delivery path instead of closing after its first send.
    tokio::spawn(async move { while cmd_rx.recv().await.is_some() {} });
    path
}

/// Bind a live sub-agent socket and expose every decoded command it receives.
pub fn spawn_subagent_socket_with_commands(
    id: &str,
) -> (std::path::PathBuf, mpsc::Receiver<String>) {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-harness-sub-{}-{}-{}",
        std::process::id(),
        id,
        n
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    spawn_command_reader(listener, cmd_tx);
    (socket_path, cmd_rx)
}

/// A `SubagentInfoEvent` carrying an explicit `socket_path` (live connection).
pub fn subagent_with_socket(
    id: &str,
    status: &str,
    wf: Option<(&str, u32, u32)>,
    socket_path: Option<std::path::PathBuf>,
) -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: socket_path.map(|p| p.to_string_lossy().into_owned()),
        parent_id: None,
        workflow: wf.map(|(mode, d, t)| SubagentWorkflow {
            mode: mode.to_string(),
            steps_completed: d,
            steps_total: t,
        }),
        read_only: false,
        runtime_backend: "local".to_string(),
        container_uuid: None,
        container_ref: None,
        container_name: None,
        repo_url: None,
        environment_id: None,
        workspace_path: None,
        environment_health: None,
        socket_mode: None,
    }
}

/// A read-only sub-agent event (`read_only: true`) for observer-marker tests
/// (#966). Otherwise identical to `subagent_with_socket`.
pub fn subagent_readonly(
    id: &str,
    status: &str,
    wf: Option<(&str, u32, u32)>,
    socket_path: Option<std::path::PathBuf>,
) -> SubagentInfoEvent {
    let mut ev = subagent_with_socket(id, status, wf, socket_path);
    ev.read_only = true;
    ev
}

/// `get_subagents`-style push of the full sub-agent list.
pub fn subagents_changed(list: Vec<SubagentInfoEvent>) -> Event {
    Event::SubagentStateChanged { subagents: list }
}

/// A `spawn` tool starting (registers the child locally as "starting").
pub fn spawn_start(id: &str) -> Event {
    Event::ToolExecutionStart {
        tool_call_id: format!("tc-spawn-{id}"),
        tool_name: "spawn".to_string(),
        args: serde_json::json!({ "agent_id": id }),
    }
}

/// An `agent_cmd await` tool starting on `id` (marks the row "awaiting").
pub fn await_start(id: &str) -> Event {
    Event::ToolExecutionStart {
        tool_call_id: format!("tc-await-{id}"),
        tool_name: "agent_cmd".to_string(),
        args: serde_json::json!({ "command": "await", "agent_id": id }),
    }
}

/// A tool finishing (clears the awaiting marker / spinner message).
pub fn tool_end(call_id: &str, tool: &str) -> Event {
    Event::ToolExecutionEnd {
        tool_call_id: call_id.to_string(),
        tool_name: tool.to_string(),
        result: serde_json::json!({ "content": [{ "type": "text", "text": "ok" }] }),
        is_error: false,
    }
}

/// A forwarded child `workflow_state` event (carries `agent_id` — must NOT
/// touch the parent's workflow bar).
pub fn forwarded_workflow(agent_id: &str, done: u32, total: u32) -> Event {
    Event::WorkflowState {
        agent_id: Some(agent_id.to_string()),
        steps: Vec::new(),
        progress: serde_json::json!({ "done": done, "total": total, "percent": done * 100 / total.max(1) }),
        active_issue: Some(serde_json::json!({ "number": 7, "title": "child" })),
        mode: Some("active".to_string()),
        active_template: None,
        available_templates: None,
    }
}

/// Drain a child-socket command receiver until the stream is quiet (#1249).
///
/// Commands arrive after several async hops (connect task → framed write →
/// reader → channel), so a multi-command batch is not all visible at once.
/// Returning on the first arrival observes a scheduler-dependent prefix; a
/// fixed wall-clock sleep can miss late duplicates or pass when nothing was
/// ever going to arrive. Keep polling until no new command arrives for a
/// short settle window after the first observation (or until the overall
/// bound elapses on a genuinely empty stream).
pub async fn drain_child_commands_until_quiet(rx: &mut mpsc::Receiver<String>) -> Vec<String> {
    const SETTLE_POLLS: usize = 15;
    const MAX_POLLS: usize = 400;
    let mut out = Vec::new();
    let mut idle = 0;
    for _ in 0..MAX_POLLS {
        let mut got = false;
        while let Ok(line) = rx.try_recv() {
            out.push(line);
            got = true;
        }
        if got {
            idle = 0;
        } else if !out.is_empty() {
            idle += 1;
            if idle >= SETTLE_POLLS {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    out
}

/// Assert no further child commands arrive after the stream has already settled.
///
/// Unlike a one-shot `timeout(recv)`, this drains until quiet again so a delayed
/// duplicate fails the test instead of slipping past a fixed window (#1249).
pub async fn assert_no_further_child_commands(rx: &mut mpsc::Receiver<String>, context: &str) {
    let late = drain_child_commands_until_quiet(rx).await;
    assert!(
        late.is_empty(),
        "{context}: unexpected further child commands: {late:?}"
    );
}

/// Wire `type` field of a drained command line, if present.
///
/// Typed deserialize (not ad-hoc `Value` key access) so this test-harness
/// helper does not inflate the #1220 feature/view raw-JSON ratchet.
pub fn child_command_type(line: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct CmdType<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
    }
    serde_json::from_str::<CmdType<'_>>(line)
        .ok()
        .map(|c| c.kind.to_string())
}

/// Accept connections on `listener` and forward each decoded command to
/// `cmd_tx`. The accept loop keeps the listener live when a test deselects and
/// reselects the same agent. The client speaks length-prefixed frames since
/// #1059; commands are read via the production deprecation-window reader,
/// skipping the empty hello frame that announces framed mode.
pub(super) fn spawn_command_reader(
    listener: tokio::net::UnixListener,
    cmd_tx: tokio::sync::mpsc::Sender<String>,
) {
    use quecto_line_io::{Incoming, PROTOCOL_FRAME_CAP_BYTES, read_frame_or_legacy_line};
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let cmd_tx = cmd_tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                while let Ok(Some(incoming)) =
                    read_frame_or_legacy_line(&mut reader, PROTOCOL_FRAME_CAP_BYTES).await
                {
                    let (Incoming::Frame(bytes) | Incoming::LegacyLine(bytes)) = incoming;
                    if bytes.is_empty() {
                        continue;
                    }
                    let line = String::from_utf8_lossy(&bytes).into_owned();
                    if cmd_tx.send(line).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
}
