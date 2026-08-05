//! Behavioural tests for the sub-agent-first persistent left panel.

// Coalescing of `◆` completion DISPLAY notes (#900) lives in its own file to
// respect the 750-line cap; wired here so it shares this module's test scope.
#[path = "app_subagent_note_coalesce_tests.rs"]
mod note_coalesce;

use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::components::chat::ChatEntry;
use crate::protocol::client::{Event, SubagentInfoEvent, SubagentWorkflow};
use crate::shell::keys::Key;

/// A `SubagentInfoEvent` with an explicit parent (for tree tests) and socket.
fn child(id: &str, status: &str, parent: Option<&str>) -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
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

async fn with_two_subagents() -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("worker", "running", Some(("active", 1, 3))),
        subagent("other", "running", Some(("active", 2, 3))),
    ]));
    h
}

#[rustfmt::skip]
fn empty_agent_end() -> Event {
    Event::AgentEnd { messages: vec![], message_refs: vec![] }
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

/// #1378: roster is UUID-keyed but the panel must paint display labels, not
/// raw UUIDs, in both the left list and the main-pane title.
#[tokio::test]
async fn panel_renders_display_label_not_uuid_key() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let uuid = "44444444-4444-4444-8444-444444444444";
    h.event(Event::SubagentStateChanged {
        subagents: vec![SubagentInfoEvent {
            agent_uuid: Some(uuid.to_string()),
            display_name: Some("reviewer".into()),
            agent_id: "reviewer".into(),
            status: "running".into(),
            last_tool: None,
            last_error: None,
            pid: 1,
            socket_path: None,
            parent_id: None,
            workflow: Some(SubagentWorkflow {
                mode: "active".into(),
                steps_completed: 1,
                steps_total: 2,
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
        }],
    });
    // Selection/identity is UUID-keyed.
    assert!(
        h.app_mut().subagents.tracked.contains_key(uuid),
        "roster must key by UUID"
    );
    h.app_mut().select_agent(Some(uuid));
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("reviewer"),
        "panel/title must paint display label:\n{frame}"
    );
    assert!(
        !frame.contains(uuid),
        "panel/title must not leak raw UUID key:\n{frame}"
    );
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
async fn esc_returns_to_master_when_subagent_idle() {
    let mut h = with_two_subagents().await;
    // Idle agents: Esc navigates back to master rather than cancelling.
    h.event(subagents_changed(vec![
        subagent("worker", "idle", Some(("active", 1, 3))),
        subagent("other", "idle", Some(("active", 2, 3))),
    ]));
    h.app_mut().select_agent(Some("worker"));
    assert_eq!(h.app_mut().active_agent_id(), Some("worker"));
    h.app_mut().handle_key(Key::Escape);
    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "Esc on an IDLE sub-agent returns the main view to the master session"
    );
}

#[tokio::test]
async fn esc_cancels_running_subagent_instead_of_returning_to_master() {
    let mut h = with_two_subagents().await; // worker tracked as "running"
    h.app_mut().select_agent(Some("worker"));
    assert_eq!(h.app_mut().active_agent_id(), Some("worker"));
    // child-feed joined mid-turn, so session.running is false — but the
    // master tracks worker as running, so Esc must CANCEL the sub-agent's work
    // (staying on it), not navigate back to master.
    assert!(
        h.app_mut().active_subagent_running(),
        "a busy sub-agent is detected via the master's tracked status"
    );
    h.app_mut().handle_key(Key::Escape);
    assert_eq!(
        h.app_mut().active_agent_id(),
        Some("worker"),
        "Esc cancels the running sub-agent (stays on it); does not return to master"
    );
}

#[tokio::test]
async fn observed_idle_overrides_stale_tracked_running_status() {
    // Once the worker's OWN stream reports it finished, that is authoritative —
    // even though the master's tracked status still lags at "running". Otherwise
    // the spinner would linger and Esc would abort instead of returning to master.
    let mut h = with_two_subagents().await; // worker tracked "running"
    h.app_mut().select_agent(Some("worker"));
    assert!(
        h.app_mut().active_subagent_running(),
        "mid-turn connect: running inferred from tracked status before any stream event"
    );
    h.app_mut()
        .route_subagent_event("worker", empty_agent_end());
    assert!(
        !h.app_mut().active_subagent_running(),
        "observed agent_end wins over the stale tracked 'running' status"
    );
    h.app_mut().handle_key(Key::Escape);
    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "Esc on a (now observed-idle) sub-agent returns to master"
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
    // Back to master: body shows master content again. Navigate directly —
    // Esc on a RUNNING sub-agent now cancels it rather than navigating.
    h.app_mut().select_agent(None);
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
    // deserializes and stores it (the value child-feed dials), via the
    // REAL wire deserializer.
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let socket = spawn_subagent_socket("worker-wire");
    let socket_path = socket.to_string_lossy().to_string();
    let line = serde_json::json!({
        "type": "subagent_state_changed",
        "subagents": [{
            "agentId": "worker",
            "status": "running",
            "pid": 7,
            "socketPath": socket_path,
        }],
    })
    .to_string();
    h.event_line(&line);
    assert_eq!(
        h.app_mut().subagent_socket_path("worker").as_deref(),
        Some(socket_path.as_str()),
        "the TUI must store the wire socketPath for child feeds"
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
    h.app_mut()
        .route_subagent_event("worker", empty_agent_end());
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
    h.event(empty_agent_end());
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    let resp = frame.find("master-response").expect("response present");
    let note = frame.find("child one done").expect("note flushed on idle");
    assert!(
        resp < note,
        "the flushed note must follow the finished response:\n{frame}"
    );
}

#[tokio::test]
async fn master_is_modeled_as_active_session_like_subagents() {
    // #828 Part 2: the master is just another `SessionView`. With no sub-agent
    // selected, the active-session accessors must resolve to the MASTER session
    // (its own chat / workflow bar / footer) — the same `active_session` path a
    // selected sub-agent takes — with `active_subagent_running` mirroring the
    // master's run state rather than being hard-coded `false`.
    let mut h = with_two_subagents().await;

    // Master active (None): the active chat is the master's, and a started turn
    // drives the unified running flag exactly as a sub-agent's would.
    assert_eq!(h.app_mut().active_agent_id(), None);
    h.app_mut().active_chat_mut().add_entry(ChatEntry::User {
        text: "master-msg".to_string(),
    });
    assert!(
        h.app_mut().active_subagent_running(),
        "AgentStart must set the master session's unified running flag"
    );

    // The master's chat is reached through the SAME active-session accessor.
    let frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("master-msg"),
        "master content renders through the unified active-session path:\n{frame}"
    );

    // Selecting a sub-agent flips the SAME accessors to that session, and the
    // master's run flag is independent of the viewed session.
    // Idle the worker so the viewed-session run flag reads false (the harness
    // tracks it running, and active_subagent_running now trusts that status).
    h.event(subagents_changed(vec![
        subagent("worker", "idle", Some(("active", 1, 3))),
        subagent("other", "running", Some(("active", 2, 3))),
    ]));
    h.app_mut().select_agent(Some("worker"));
    assert_eq!(h.app_mut().active_agent_id(), Some("worker"));
    assert!(
        !h.app_mut().active_subagent_running(),
        "the freshly-selected idle sub-agent session is not running"
    );

    // Esc returns to the master session and its running flag is intact.
    h.press(Key::Escape);
    assert_eq!(h.app_mut().active_agent_id(), None);
    assert!(
        h.app_mut().active_subagent_running(),
        "returning to master restores its still-running unified flag"
    );
}

#[tokio::test]
async fn container_panel_probe_drives_real_roster_render_navigation_and_details() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let mut solo = child("alpha", "running", None);
    solo.runtime_backend = "container".into();
    solo.container_uuid = Some("uuid-solo".into());
    solo.container_ref = Some("C1".into());
    solo.environment_health = Some("healthy".into());
    let mut beta = child("beta", "running", None);
    beta.runtime_backend = "container".into();
    beta.container_uuid = Some("uuid-shared".into());
    beta.container_ref = Some("C2".into());
    beta.container_name = Some("platform-q-ai/quecto".into());
    beta.repo_url = Some("platform-q-ai/quecto".into());
    beta.environment_id = Some("env-shared".into());
    beta.workspace_path = Some("/workspace/quecto".into());
    beta.environment_health = Some("healthy".into());
    let mut gamma = beta.clone();
    gamma.agent_id = "gamma".into();
    h.event(Event::SubagentStateChanged {
        subagents: vec![solo, beta, gamma],
    });
    let frame = strip_ansi(
        &h.app_mut()
            .render_subagent_panel(80, 10, tokio::time::Instant::now())
            .join("\n"),
    );
    assert!(
        frame.contains("alpha · C1"),
        "solo container row must expose ref inline:\n{frame}"
    );
    assert!(
        frame.contains("C2 platform-q-ai/quecto"),
        "shared env group row must render:\n{frame}"
    );
    assert!(
        frame.contains("beta") && frame.contains("gamma"),
        "shared members must render under group:\n{frame}"
    );
    h.app_mut().panel_highlight_row(2);
    h.app_mut().commit_panel_selection();
    let selected_frame = strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        selected_frame.contains("repo:platform-q-ai/quecto"),
        "committing shared environment should render details in the main pane:\n{selected_frame}"
    );
    let detail = crate::agents::container_panel::environment_title(
        &h.app_mut().subagents.tracked.get("beta").unwrap().info,
    );
    assert!(
        detail.contains("repo:platform-q-ai/quecto"),
        "selected env details show repo:\n{detail}"
    );
    assert!(
        detail.contains("runtime:container"),
        "selected env details show runtime:\n{detail}"
    );
    assert!(
        detail.contains("workspace:/workspace/quecto"),
        "selected env details show workspace:\n{detail}"
    );
    assert!(
        detail.contains("status:healthy"),
        "selected env details show health:\n{detail}"
    );
}
