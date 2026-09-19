//! #1466 fix-pass tests that still pin single-session behaviour: `/resume`
//! recency order, and dead/detached sub-agents staying visible with sends
//! never silently swallowed. (The tab-bar, chord, workspace-manifest and
//! background-tab cases left with the multi-tab machinery, #2044.)

use super::*;
use crate::shell::terminal::Terminal;

fn headless_app() -> App {
    let client = crate::protocol::client::Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    let mut app = App::new(term, client);
    app.suppress_paint = true;
    app
}

fn subagent_info(id: &str, status: &str) -> crate::protocol::client::SubagentInfoEvent {
    crate::protocol::client::SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        compact: false,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: None,
        environment: None,
    }
}

// ── /resume recency sort ─────────────────────────────────────────────────

#[test]
fn resume_selector_sorts_sessions_by_last_active_descending() {
    let mut app = headless_app();
    let data = serde_json::json!({
        "sessions": [
            { "key": "s-old", "title": "older session", "messageCount": 3,
              "updatedUnixSecs": 1_000 },
            { "key": "s-new", "title": "newer session", "messageCount": 5,
              "updatedUnixSecs": 2_000 },
        ]
    });
    app.open_resume_selector(&data);
    let sel = app
        .ac()
        .sessions
        .resume_selector
        .as_ref()
        .expect("selector");
    let values: Vec<_> = sel
        .items_for_tests()
        .iter()
        .map(|i| i.value.clone())
        .collect();
    assert_eq!(
        values[0], "session:s-new",
        "bare sessions must also list most-recently-active first; got {values:?}"
    );
}

// ── Dead sub-agents are visible, sends are never swallowed ───────

#[test]
fn detached_and_dead_subagent_names_are_visually_distinct() {
    use super::app_subagent_panel::controller_subagent_panel_helpers::status_colored_name;
    use crate::components::theme;
    assert_eq!(
        status_colored_name("detached", "w1"),
        theme::dim("w1"),
        "a detached roster entry must render dimmed, not like a live agent \
         (#1461 liveness states)"
    );
    assert_eq!(
        status_colored_name("dead", "w1"),
        theme::red("w1"),
        "a dead roster entry must render red, not like a live agent \
         (#1461 liveness states)"
    );
}

#[tokio::test]
async fn sending_to_an_unattached_subagent_surfaces_a_visible_error() {
    let mut app = headless_app();
    // Rehydrated roster entry that is detached AND unreachable: no usable
    // child socket exists, so attach-on-demand (#1466 round 2) has no route
    // and the send must keep erroring. The detached-but-REACHABLE side is
    // pinned by the sub-agent delivery scenarios (live socket → delivered).
    app.update_subagent_bar(vec![subagent_info("w1", "detached")]);
    app.select_agent(Some("w1"));
    // Note: a feed channel may exist and even accept the enqueue — with a
    // dead/detached child nothing consumes it, which is exactly the silent
    // swallow being fixed. Liveness must be judged from the roster state.
    app.handle_submit("hello there");

    // The outcome must SPECIFICALLY reference the failed delivery and the
    // agent — an incidental unrelated notification must not pass.
    let status = app
        .active_session()
        .chat
        .last_status_text()
        .unwrap_or("")
        .to_string();
    let last_note = app
        .notifications
        .messages()
        .last()
        .cloned()
        .unwrap_or_default();
    let surfaced = |s: &str| s.contains("not delivered") && s.contains("w1");
    assert!(
        surfaced(&status) || surfaced(&last_note),
        "a message to a dead/unattached sub-agent must surface a delivery \
         failure naming the agent; last status={status:?}, last \
         notification={last_note:?}"
    );
}
