//! RED-phase tests for #802: full view + interaction parity for selected
//! sub-agents, the Tab focus model, and the focus-highlighted divider.
//!
//! These drive the *real* render/key path through the headless harness and the
//! `App` key handler. They are written against the intended behaviour and are
//! expected to FAIL until #802 is implemented (RED). They compile against the
//! current API and assert on observable outputs (active session, drained
//! commands, rendered frame), so no production code is added by this phase.

use super::app_methods::strip_ansi;
use super::keys::Key;
use super::tui_harness::*;
use crate::infrastructure::client::Event;

/// Build a harness with N tracked sub-agents (panel visible) named a1..aN.
async fn harness_with_subagents(n: usize) -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let mut infos = Vec::new();
    for i in 1..=n {
        let id = format!("a{i}");
        h.event(spawn_start(&id));
        // Give each sub-agent a live, drained socket so connect-on-commit
        // succeeds and the per-child command channel stays live — routing tests
        // then exercise the real `try_send` delivery path (#804 review).
        let socket = spawn_subagent_socket(&id);
        infos.push(subagent_with_socket(
            &id,
            "running",
            Some(("active", 0, 3)),
            Some(socket),
        ));
    }
    h.event(subagents_changed(infos));
    h
}

fn frame_text(h: &mut TuiHarness) -> String {
    h.app_mut()
        .compose_frame()
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Goal 4: focus-highlighted divider ───────────────────────────────────

#[tokio::test]
async fn divider_rule_drawn_between_panel_and_body() {
    // Goal 4: a vertical rule must separate the panel from the body. Today the
    // panel cell is string-concatenated onto each row with no separator.
    let mut h = harness_with_subagents(2).await;
    let frame = frame_text(&mut h);
    assert!(
        frame.contains('│'),
        "expected a vertical divider between panel and body, got:\n{frame}"
    );
}

// ── Goal 3: Tab focus model ──────────────────────────────────────────────

#[tokio::test]
async fn panel_movement_moves_highlight_without_committing() {
    // Tab focuses the panel; Down then only moves the highlight — the active
    // session must NOT change on mere movement (commit-on-confirm).
    let mut h = harness_with_subagents(2).await;
    h.app_mut().handle_key(Key::Tab);
    assert_eq!(
        h.app_mut().focus_region(),
        super::Focus::Panel,
        "Tab with no popup must move focus to the panel"
    );
    let before = h.app_mut().panel_highlight_index();
    h.app_mut().handle_key(Key::Down);
    assert_eq!(
        h.app_mut().panel_highlight_index(),
        before + 1,
        "Down must advance the panel highlight"
    );
    assert_eq!(
        h.app_mut().active_agent_id(),
        None,
        "highlight movement must not switch the active session"
    );
    assert_eq!(
        h.app_mut().focus_region(),
        super::Focus::Panel,
        "focus must remain on the panel during movement"
    );
}

#[tokio::test]
async fn tab_completes_while_autocomplete_open() {
    // Goal 3: Tab is context-sensitive. With an autocomplete popup open it must
    // keep completing and NOT move focus to the panel.
    let mut h = harness_with_subagents(1).await;
    h.app_mut().handle_key(Key::Char('/'));
    h.app_mut().handle_key(Key::Tab);
    assert_eq!(
        h.app_mut().focus_region(),
        super::Focus::Input,
        "Tab must complete (not toggle focus) while a popup is open"
    );
}

#[tokio::test]
async fn enter_commit_returns_focus_to_input() {
    // Goal 3: Enter commits the highlighted agent AND returns focus to the input.
    let mut h = harness_with_subagents(2).await;
    h.app_mut().handle_key(Key::Tab);
    h.app_mut().handle_key(Key::Down);
    h.app_mut().handle_key(Key::Enter);
    assert_eq!(h.app_mut().active_agent_id(), Some("a1"));
    assert_eq!(
        h.app_mut().focus_region(),
        super::Focus::Input,
        "commit must return focus to the input"
    );
}

#[tokio::test]
async fn digit_jump_then_enter_commits_that_row() {
    // In panel focus, digit "2" jumps the highlight to row 2 (a1, since row 1 is
    // the master), and Enter commits it. Today the digit is typed into the
    // editor and Enter submits it as a master prompt — the active session never
    // changes.
    let mut h = harness_with_subagents(2).await;
    h.app_mut().handle_key(Key::Tab);
    h.app_mut().handle_key(Key::Char('2'));
    h.app_mut().handle_key(Key::Enter);
    assert_eq!(
        h.app_mut().active_agent_id(),
        Some("a1"),
        "digit-jump + Enter must commit the numbered agent row"
    );
    // The digit must not have leaked into the editor and been sent as a prompt.
    let cmds = h.drain_commands().await;
    assert!(
        !cmds
            .iter()
            .any(|c| c.contains("\"prompt\"") && c.contains("\"2\"")),
        "digit jump must not submit \"2\" as a master prompt: {cmds:?}"
    );
}

#[tokio::test]
async fn esc_in_panel_focus_keeps_active_selection() {
    // Esc while the panel is focused cancels back to the input WITHOUT changing
    // the active session. Today Esc on a sub-agent view returns to the master.
    let mut h = harness_with_subagents(2).await;
    h.app_mut().select_agent(Some("a1"));
    h.app_mut().handle_key(Key::Tab);
    h.app_mut().handle_key(Key::Down);
    h.app_mut().handle_key(Key::Escape);
    assert_eq!(
        h.app_mut().active_agent_id(),
        Some("a1"),
        "Esc in panel focus must not change the active session"
    );
    assert_eq!(
        h.app_mut().focus_region(),
        super::Focus::Input,
        "Esc in panel focus must return focus to the input"
    );
}

// ── Goal 2: interaction parity (steer the active session) ────────────────

#[tokio::test]
async fn send_routes_to_active_subagent_not_master() {
    // With a sub-agent active, a sent prompt must target THAT agent's
    // connection — it must NOT be sent to the master connection. Today
    // handle_submit always sends to the master client.
    let mut h = harness_with_subagents(1).await;
    h.app_mut().select_agent(Some("a1"));
    h.app_mut().handle_submit("steer please");
    // POSITIVE: the prompt lands in a1's OWN session transcript (observable in
    // the body), proving it was routed to that session — not silently dropped.
    let frame = frame_text(&mut h);
    assert!(
        frame.contains("steer please"),
        "prompt must land in the active sub-agent's session body, got:\n{frame}"
    );
    // NEGATIVE: it must NOT be sent to the master connection.
    let cmds = h.drain_commands().await;
    assert!(
        !cmds
            .iter()
            .any(|c| c.contains("\"prompt\"") && c.contains("steer please")),
        "prompt must route to the active sub-agent, not the master: {cmds:?}"
    );
}

#[tokio::test]
async fn abort_targets_active_session_not_master() {
    // Goal 2 / AC: abort targets the active session. With a sub-agent active,
    // handle_abort must NOT send an abort to the master connection.
    let mut h = harness_with_subagents(1).await;
    h.app_mut().select_agent(Some("a1"));
    h.app_mut().handle_abort();
    let cmds = h.drain_commands().await;
    assert!(
        !cmds.iter().any(|c| c.contains("\"abort\"")),
        "abort must target the active sub-agent, not the master: {cmds:?}"
    );
}

// ── Goal 1: full VIEW parity (per-session workflow render) ───────────────

#[tokio::test]
async fn active_subagent_renders_its_own_workflow_bar() {
    // A forwarded workflow_state routed into a sub-agent's session must render
    // as THAT agent's own workflow/phase bar when it is the active session.
    // Today SessionView holds only `chat`, so the active sub-agent shows no
    // workflow bar of its own.
    let mut h = harness_with_subagents(1).await;
    h.app_mut()
        .route_subagent_event("a1", forwarded_workflow("a1", 2, 5));
    h.app_mut().select_agent(Some("a1"));
    let frame = frame_text(&mut h);
    // The sub-agent-first main pane (#820) shows the active agent's workflow as a
    // boxed bar whose title line carries its OWN active issue (`#7`), unique to
    // a1's forwarded workflow — proving the bar is THIS agent's, not chrome.
    assert!(
        frame.contains("#7"),
        "active sub-agent must render its own workflow bar (active issue), got:\n{frame}"
    );
    // Switching back to master must NOT show the sub-agent's workflow issue.
    h.app_mut().select_agent(None);
    let master = frame_text(&mut h);
    assert!(
        !master.contains("#7"),
        "master view must not show the sub-agent's workflow bar, got:\n{master}"
    );
}

#[tokio::test]
async fn divider_brightens_on_focused_pane() {
    // Goal 4: the divider is bright/colored on the focused pane and dim on the
    // other, so it signals which pane has focus. Compare the RAW (ANSI-bearing)
    // frame across a focus toggle: the divider's styling must change.
    let mut h = harness_with_subagents(2).await;
    let input_focus: String = h.app_mut().compose_frame().join("\n");
    h.app_mut().handle_key(Key::Tab); // focus the panel
    let panel_focus: String = h.app_mut().compose_frame().join("\n");
    assert_ne!(
        input_focus, panel_focus,
        "the divider styling must differ between input-focused and panel-focused"
    );
}

// ── Goal 1: full VIEW parity (per-session FOOTER render) ─────────────────

#[tokio::test]
async fn footer_reflects_active_session() {
    // #805 Gap 1 / Criterion 1: the footer's context-window / cost / model
    // gauges must reflect the ACTIVE session. A selected sub-agent must show
    // ITS OWN footer (fed by its forwarded get_state / turn_end / session-stats
    // events), and switching back to master must restore the master's footer.
    //
    // Today `compose_bottom` always renders `self.footer` (the master's), and
    // `route_subagent_event` never updates a sub-agent's footer — so a selected
    // sub-agent shows the MASTER's gauges. These assertions are expected to FAIL
    // until the per-session footer lands (RED).
    let mut h = harness_with_subagents(1).await;

    // Master footer: distinct model + context usage (50.0% of a 200k window).
    h.event(Event::Response {
        id: None,
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({
            "model": "mastrmdl",
            "maxContextTokens": 200_000,
        })),
        error: None,
    });
    h.event(Event::TurnEnd {
        message: serde_json::json!({
            "contextTokens": 100_000,
            "maxContextTokens": 200_000,
        }),
        tool_results: vec![],
    });

    // Sub-agent a1's OWN footer, delivered over its direct connection: a
    // distinct model, a different context usage (25.0%), and a session cost.
    h.app_mut().route_subagent_event(
        "a1",
        Event::Response {
            id: None,
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": "subbymdl",
                "maxContextTokens": 200_000,
            })),
            error: None,
        },
    );
    h.app_mut().route_subagent_event(
        "a1",
        Event::TurnEnd {
            message: serde_json::json!({
                "contextTokens": 50_000,
                "maxContextTokens": 200_000,
            }),
            tool_results: vec![],
        },
    );
    h.app_mut().route_subagent_event(
        "a1",
        Event::Response {
            id: None,
            command: "get_session_stats".into(),
            success: true,
            data: Some(serde_json::json!({
                "cost": 0.0777,
                "contextTokens": 50_000,
                "maxContextTokens": 200_000,
            })),
            error: None,
        },
    );

    // With the sub-agent active the footer must show ITS gauges, not master's.
    h.app_mut().select_agent(Some("a1"));
    let on_sub = frame_text(&mut h);
    assert!(
        on_sub.contains("50k"),
        "active sub-agent footer must show its own context usage (50k), got:\n{on_sub}"
    );
    assert!(
        on_sub.contains("subbymdl"),
        "active sub-agent footer must show its own model, got:\n{on_sub}"
    );
    assert!(
        on_sub.contains("$0.0777"),
        "active sub-agent footer must show its own session cost, got:\n{on_sub}"
    );
    assert!(
        !on_sub.contains("mastrmdl"),
        "the master's model must NOT show while a sub-agent is active, got:\n{on_sub}"
    );
    assert!(
        !on_sub.contains("100k"),
        "the master's context usage (100k) must NOT show while a sub-agent is active, got:\n{on_sub}"
    );

    // Switching back to master must restore the master's footer.
    h.app_mut().select_agent(None);
    let on_master = frame_text(&mut h);
    assert!(
        on_master.contains("100k") && on_master.contains("mastrmdl"),
        "master footer must be restored (100k + mastrmdl), got:\n{on_master}"
    );
    assert!(
        !on_master.contains("50k"),
        "the sub-agent's context usage (50k) must NOT show while master is active, got:\n{on_master}"
    );
    assert!(
        !on_master.contains("subbymdl") && !on_master.contains("$0.0777"),
        "the sub-agent's footer must NOT show while master is active, got:\n{on_master}"
    );
}
