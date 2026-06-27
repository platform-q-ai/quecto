//! RED-phase behavioural tests for the sub-agent-first persistent left panel
//! and multi-session switching (#800), driven through the headless render
//! harness and the real key handler.
//!
//! These pin the acceptance criteria from the issue:
//!   * the panel appears in the normal view once a sub-agent exists (no
//!     separate mode), and is absent — layout unchanged — with none;
//!   * the panel lists Master + sub-agents, nesting grandchildren under their
//!     parent as a tree via `parent_id`;
//!   * ↑/↓ moves selection and switches `active_agent_id` to that agent's
//!     session; Esc returns to the master;
//!   * switching preserves each session; an exited agent's session remains
//!     viewable per the retention policy;
//!   * `compose_frame` stays render-idempotent (no flash) with the panel.
//!
//! They drive not-yet-existing `App` API on purpose (TDD RED).

use super::tui_harness::*;
use crate::infrastructure::client::{Event, SubagentInfoEvent, SubagentWorkflow};
use crate::interface::ansi::strip_ansi;
use crate::interface::keys::Key;

/// A `SubagentInfoEvent` with an explicit parent (for tree tests) and socket.
fn child(id: &str, status: &str, parent: Option<&str>) -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: None,
        parent_id: parent.map(|p| p.to_string()),
        workflow: Some(SubagentWorkflow {
            mode: "active".to_string(),
            steps_completed: 1,
            steps_total: 3,
        }),
    }
}

async fn with_two_subagents() -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("worker", "running", Some(("active", 1, 3))),
        subagent("other", "running", Some(("active", 2, 3))),
    ]));
    h
}

#[tokio::test]
async fn panel_always_visible_even_without_subagents() {
    // Sub-agent-first default (#820): the panel is ALWAYS on once connected,
    // with the Master pinned as the top row even when no sub-agents exist.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    assert!(
        h.app_mut().subagent_panel_visible(),
        "the left panel must be always visible (Master row), not gated on sub-agents"
    );
}

#[tokio::test]
async fn panel_appears_on_subagent_spawn() {
    let mut h = with_two_subagents().await;
    assert!(
        h.app_mut().subagent_panel_visible(),
        "spawning a sub-agent makes the persistent left panel appear"
    );
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("Master Agent"),
        "panel pins Master Agent at top:\n{frame}"
    );
    assert!(frame.contains("worker"), "panel lists sub-agents:\n{frame}");
}

#[tokio::test]
async fn master_is_active_by_default() {
    let mut h = with_two_subagents().await;
    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "master (None) is the active session until the user selects a child"
    );
}

#[tokio::test]
async fn selecting_a_subagent_switches_active_session() {
    let mut h = with_two_subagents().await;
    h.app_mut().select_agent(Some("worker"));
    assert_eq!(
        h.app_mut().active_agent_id(),
        Some("worker"),
        "selecting a sub-agent makes it the active session"
    );
}

#[tokio::test]
async fn esc_returns_to_master() {
    let mut h = with_two_subagents().await;
    h.app_mut().select_agent(Some("worker"));
    assert_eq!(h.app_mut().active_agent_id(), Some("worker"));
    h.app_mut().handle_key(Key::Escape);
    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "Esc returns the main view to the master session"
    );
}

#[tokio::test]
async fn grandchild_renders_indented_under_its_parent() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        child("worker", "running", None),
        child("grandchild", "running", Some("worker")),
    ]));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    // The grandchild row must be more deeply indented than its parent row.
    let indent = |needle: &str| {
        frame
            .lines()
            .find(|l| l.contains(needle))
            .map(|l| l.len() - l.trim_start().len())
            .unwrap_or_else(|| panic!("row for {needle} not found in:\n{frame}"))
    };
    assert!(
        indent("grandchild") > indent("worker"),
        "grandchild must be indented deeper than its parent (tree):\n{frame}"
    );
}

#[tokio::test]
async fn three_level_tree_nests_grandchild_under_child_in_order() {
    // master → childA → grandchildB must render childA at depth 1 and
    // grandchildB at depth 2 (deeper indent), with grandchildB BELOW childA.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        child("childA", "running", None),
        child("grandchildB", "running", Some("childA")),
    ]));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    let row_index = |needle: &str| {
        frame
            .lines()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("row for {needle} not found in:\n{frame}"))
    };
    let indent = |needle: &str| {
        frame
            .lines()
            .find(|l| l.contains(needle))
            .map(|l| l.len() - l.trim_start().len())
            .unwrap_or_else(|| panic!("row for {needle} not found in:\n{frame}"))
    };
    assert!(
        indent("grandchildB") > indent("childA"),
        "grandchildB must be indented deeper than childA:\n{frame}"
    );
    assert!(
        row_index("grandchildB") > row_index("childA"),
        "grandchildB must render BELOW its parent childA, never above:\n{frame}"
    );
}

#[tokio::test]
async fn partial_child_view_push_does_not_evict_intermediate_parent() {
    // Regression (grandchild-at-depth-1 bug): the master push carries the full
    // tree (childA + grandchildB), then a forwarded child's-eye-view push lists
    // ONLY grandchildB (childA's own child) and omits childA itself. A naive
    // full-replace would evict childA, re-rooting grandchildB to depth 1 above
    // its parent. The roster must instead preserve childA so nesting holds.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        child("childA", "running", None),
        child("grandchildB", "running", Some("childA")),
    ]));
    // The partial push: only grandchildB, parent still childA.
    h.event(subagents_changed(vec![child(
        "grandchildB",
        "running",
        Some("childA"),
    )]));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    let row_index = |needle: &str| {
        frame
            .lines()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("row for {needle} not found in:\n{frame}"))
    };
    let indent = |needle: &str| {
        frame
            .lines()
            .find(|l| l.contains(needle))
            .map(|l| l.len() - l.trim_start().len())
            .unwrap_or_else(|| panic!("row for {needle} not found in:\n{frame}"))
    };
    // childA must NOT have been evicted by the partial push.
    assert!(
        frame.lines().any(|l| l.contains("childA")),
        "the intermediate parent childA must survive a partial child-view push:\n{frame}"
    );
    assert!(
        indent("grandchildB") > indent("childA"),
        "grandchildB must stay nested under childA after a partial push:\n{frame}"
    );
    assert!(
        row_index("grandchildB") > row_index("childA"),
        "grandchildB must stay BELOW childA after a partial push:\n{frame}"
    );
}

#[tokio::test]
async fn compose_frame_with_panel_is_idempotent() {
    let mut h = with_two_subagents().await;
    h.app_mut().select_agent(Some("worker"));
    let a = h.app_mut().compose_frame();
    let b = h.app_mut().compose_frame();
    assert_eq!(a, b, "compose_frame must be render-idempotent (no flash)");
}

#[tokio::test]
async fn exited_subagent_session_is_retained_and_selectable() {
    let mut h = with_two_subagents().await;
    // Select the worker and give its session identifiable history.
    h.app_mut().select_agent(Some("worker"));
    h.app_mut().route_subagent_event(
        "worker",
        Event::Token {
            token: "PERSISTME".into(),
        },
    );
    // ...then it exits and drops out of the live list.
    h.event(subagents_changed(vec![subagent(
        "other",
        "running",
        Some(("active", 2, 3)),
    )]));
    assert!(
        h.app_mut()
            .retained_session_ids()
            .iter()
            .any(|id| id == "worker"),
        "an exited sub-agent's session must be retained for later viewing"
    );
    // Reselect it and confirm its history still RENDERS (not just that the id
    // survives): the retained session keeps its content.
    h.app_mut().select_agent(Some("worker"));
    assert_eq!(h.app_mut().active_agent_id(), Some("worker"));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("PERSISTME"),
        "exited agent's retained session must still render its history:\n{frame}"
    );
}

#[tokio::test]
async fn selecting_a_subagent_renders_its_session_body() {
    // The crux of #800: the body must render the SELECTED agent's full live
    // session — chat/token stream + tool execution — not just flip a scalar.
    let mut h = with_two_subagents().await;
    // Master content (active == None) lands in the master session.
    h.event(Event::Token {
        token: "MASTER_ONLY_REPLY".into(),
    });
    // Switch to the worker and feed ITS live stream: a streamed token and a
    // live tool execution, exactly as its direct connection would deliver them.
    h.app_mut().select_agent(Some("worker"));
    h.app_mut().route_subagent_event(
        "worker",
        Event::Token {
            token: "WORKER_STREAMED_REPLY".into(),
        },
    );
    h.app_mut().route_subagent_event(
        "worker",
        Event::ToolExecutionStart {
            tool_call_id: "tc1".into(),
            tool_name: "worker_tool".into(),
            args: serde_json::json!({"cmd": "ls"}),
        },
    );
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("WORKER_STREAMED_REPLY"),
        "selected agent's streamed token must render in the body:\n{frame}"
    );
    assert!(
        frame.contains("worker_tool"),
        "selected agent's live tool execution must render in the body:\n{frame}"
    );
    assert!(
        !frame.contains("MASTER_ONLY_REPLY"),
        "the master's content must NOT show while a sub-agent is active:\n{frame}"
    );
}

#[tokio::test]
async fn switching_updates_body_and_preserves_each_session() {
    let mut h = with_two_subagents().await;
    // Distinguishable content per session.
    h.event(Event::Token {
        token: "MASTERWORK".into(),
    });
    h.app_mut().select_agent(Some("worker"));
    h.app_mut().route_subagent_event(
        "worker",
        Event::Token {
            token: "WORKERWORK".into(),
        },
    );
    let on_worker = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        on_worker.contains("WORKERWORK"),
        "worker body:\n{on_worker}"
    );
    assert!(
        !on_worker.contains("MASTERWORK"),
        "switching to worker must replace the master body:\n{on_worker}"
    );
    // Esc back to master: body shows master content again.
    h.app_mut().handle_key(Key::Escape);
    let on_master = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        on_master.contains("MASTERWORK"),
        "master body:\n{on_master}"
    );
    assert!(
        !on_master.contains("WORKERWORK"),
        "returning to master must not show the worker body:\n{on_master}"
    );
    // Reselect worker: its session (scroll/history) is preserved.
    h.app_mut().select_agent(Some("worker"));
    let back_on_worker = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        back_on_worker.contains("WORKERWORK"),
        "each session must preserve its own history across switches:\n{back_on_worker}"
    );
}

#[tokio::test]
async fn active_resets_to_master_when_viewed_agent_leaves_the_list() {
    // #800 review: when the viewed sub-agent exits and drops out of the live
    // list, the panel only lists tracked agents, so body and panel must agree —
    // the active session falls back to the master.
    let mut h = with_two_subagents().await;
    h.app_mut().select_agent(Some("worker"));
    assert_eq!(h.app_mut().active_agent_id(), Some("worker"));
    // Only "worker" exits; "other" remains so the panel stays visible.
    h.event(subagents_changed(vec![subagent(
        "other",
        "running",
        Some(("active", 2, 3)),
    )]));
    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "a viewed agent leaving the live list must reset the body to the master"
    );

    // And when the *only* agent leaves (panel vanishes), active must still be
    // the master — never a dangling sub-agent id with no panel.
    h.app_mut().select_agent(Some("other"));
    assert_eq!(h.app_mut().active_agent_id(), Some("other"));
    h.event(subagents_changed(vec![]));
    // Sub-agent-first (#820): the panel stays on (Master row) even with none.
    assert!(h.app_mut().subagent_panel_visible());
    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "with the panel gone the body must be the master, not a hidden session"
    );
}

#[tokio::test]
async fn stale_event_for_untracked_agent_does_not_create_a_session() {
    // #800 review: a frame queued from a torn-down connection must not
    // resurrect (or newly create) a session for an agent that is neither
    // tracked nor retained.
    let mut h = with_two_subagents().await;
    h.app_mut().route_subagent_event(
        "ghost",
        Event::Token {
            token: "STALE".into(),
        },
    );
    assert!(
        !h.app_mut()
            .retained_session_ids()
            .iter()
            .any(|id| id == "ghost"),
        "a stale event for an untracked agent must be dropped, not create a session"
    );
}

#[tokio::test]
async fn tui_consumes_socket_path_from_the_wire() {
    // The single sanctioned kernel change surfaces `socketPath`; prove the TUI
    // deserializes and stores it (the value connect-on-select dials), via the
    // REAL wire deserializer.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(
        r#"{"type":"subagent_state_changed","subagents":[{"agentId":"worker","status":"running","pid":7,"socketPath":"/run/quecto/worker.sock"}]}"#,
    );
    assert_eq!(
        h.app_mut().subagent_socket_path("worker").as_deref(),
        Some("/run/quecto/worker.sock"),
        "the TUI must store the wire socketPath for connect-on-select"
    );
}

// ── #828 Part 1: full conversation backfill reconcile ────────────────────────

/// Build the `get_messages` backfill Response the connect-on-select path
/// requests, carrying a user/assistant transcript that pre-dates the live
/// stream.
fn backfill_history(pairs: &[(&str, &str)]) -> Event {
    let messages: Vec<serde_json::Value> = pairs
        .iter()
        .flat_map(|(u, a)| {
            [
                serde_json::json!({ "role": "user", "content": u }),
                serde_json::json!({ "role": "assistant", "content": a }),
            ]
        })
        .collect();
    Event::Response {
        id: Some("subagent-history".into()),
        command: "get_messages".into(),
        success: true,
        data: Some(serde_json::json!({ "messages": messages })),
        error: None,
    }
}

#[tokio::test]
async fn backfill_prepends_history_and_preserves_live_tokens() {
    // #828 Part 1: a busy child streams live tokens BEFORE its dispatch loop can
    // answer the connect-on-select get_messages backfill. When the backfill
    // finally arrives it must reconcile as history PREPENDED above the live
    // content — never a wholesale replace that drops the live tokens.
    let mut h = with_two_subagents().await;
    h.app_mut().select_agent(Some("worker"));
    // Live stream arrives first (child is mid-turn).
    h.app_mut().route_subagent_event(
        "worker",
        Event::Token {
            token: "LIVE_AFTER_SELECT".into(),
        },
    );
    // ...then the backfill history is finally answered.
    h.app_mut().route_subagent_event(
        "worker",
        backfill_history(&[("earlier question", "earlier answer")]),
    );

    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("earlier question") && frame.contains("earlier answer"),
        "backfilled history must render in the session:\n{frame}"
    );
    assert!(
        frame.contains("LIVE_AFTER_SELECT"),
        "the late backfill must NOT drop the live token streamed before it:\n{frame}"
    );
    let hist = frame.find("earlier answer").expect("history present");
    let live = frame.find("LIVE_AFTER_SELECT").expect("live present");
    assert!(
        hist < live,
        "history must be PREPENDED above the live content:\n{frame}"
    );
}

#[tokio::test]
async fn backfill_is_idempotent_and_does_not_duplicate_history() {
    // A re-delivered backfill (e.g. a reconnect) must not duplicate prior
    // history nor lose live content.
    let mut h = with_two_subagents().await;
    h.app_mut().select_agent(Some("worker"));
    h.app_mut().route_subagent_event(
        "worker",
        Event::Token {
            token: "LIVEONE".into(),
        },
    );
    let backfill = backfill_history(&[("the question", "the answer")]);
    h.app_mut().route_subagent_event("worker", backfill.clone());
    h.app_mut().route_subagent_event("worker", backfill);

    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert_eq!(
        frame.matches("the answer").count(),
        1,
        "re-delivered backfill must not duplicate history:\n{frame}"
    );
    assert!(
        frame.contains("LIVEONE"),
        "re-delivered backfill must not drop live content:\n{frame}"
    );
}

#[tokio::test]
async fn deferred_subagent_note_buffer_is_capped() {
    // #828 Part 2 NIT: the per-session deferred-note buffer must be defensively
    // capped so a chatty grandchild during a long parent turn cannot grow it
    // without bound. Push far more notes than any sane cap while the child is
    // mid-turn, then let it go idle and count what flushes.
    let mut h = with_two_subagents().await;
    h.app_mut().select_agent(Some("worker"));
    // Keep the child mid-turn so notes are DEFERRED, not rendered immediately.
    h.app_mut()
        .route_subagent_event("worker", Event::AgentStart);
    const PUSHED: usize = 1000;
    for i in 0..PUSHED {
        h.app_mut().route_subagent_event(
            "worker",
            Event::SubagentNotification {
                agent_id: "grandchild".into(),
                sequence: i as u64,
                message: format!("note number {i}"),
            },
        );
    }
    // Child goes idle: deferred notes flush into the chat. Count the resulting
    // chat entries directly (not the clipped viewport) so this asserts the
    // BUFFER cap, not the screen height.
    h.app_mut().route_subagent_event(
        "worker",
        Event::AgentEnd {
            messages: Vec::new(),
        },
    );
    let entries = h
        .app_mut()
        .session_chat_entry_count("worker")
        .expect("worker session exists");
    // Pin the ACTUAL cap, not merely `< PUSHED` (a cap of 999 would be
    // effectively unbounded): no more than `DEFERRED_NOTE_CAP` notes survive
    // (the session had no prior chat entries, so the cap is the whole count).
    assert!(
        entries <= super::app_subagent_stream::DEFERRED_NOTE_CAP,
        "the deferred-note buffer must be capped at {}, but {entries} chat \
         entries flushed from {PUSHED} pushed notes (unbounded growth)",
        super::app_subagent_stream::DEFERRED_NOTE_CAP
    );
    // Eviction policy is locked too: the NEWEST notes survive, the oldest are
    // dropped — a flushed-but-wrong-order buffer is a silent UX regression.
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains(&format!("note number {}", PUSHED - 1)),
        "the newest note must survive the cap:\n{frame}"
    );
    assert!(
        !frame.contains("note number 0 "),
        "the oldest note must be evicted under the cap:\n{frame}"
    );
}

#[tokio::test]
async fn master_defers_and_flushes_notes_like_a_session() {
    // #828 Part 2: the master shares the ONE defer/flush policy with sub-agent
    // sessions. A note arriving mid-turn is deferred (never split into the
    // in-flight response) and flushed AFTER the finished response once idle.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(Event::Token {
        token: "master-response".into(),
    });
    h.event(Event::SubagentNotification {
        agent_id: "child".into(),
        sequence: 0,
        message: "child one done".into(),
    });
    let mid = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        !mid.contains("child one done"),
        "a note must be DEFERRED while the master is mid-turn:\n{mid}"
    );
    h.event(Event::AgentEnd {
        messages: Vec::new(),
    });
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    let resp = frame.find("master-response").expect("response present");
    let note = frame.find("child one done").expect("note flushed on idle");
    assert!(
        resp < note,
        "the flushed note must follow the finished response:\n{frame}"
    );
}

#[tokio::test]
async fn backfill_into_idle_session_renders_full_history_in_order() {
    // #828 Part 1 (idle path): selecting an IDLE sub-agent with no live stream
    // in flight must still backfill its FULL prior conversation, in order —
    // never an empty session or mis-ordered history.
    let mut h = with_two_subagents().await;
    h.app_mut().select_agent(Some("worker"));
    h.app_mut().route_subagent_event(
        "worker",
        backfill_history(&[("first question", "first answer")]),
    );

    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    let q = frame
        .find("first question")
        .expect("history question present");
    let a = frame.find("first answer").expect("history answer present");
    assert!(
        q < a,
        "idle backfill must render history in order (question above answer):\n{frame}"
    );
}
