//! Environment visibility and grouping in the sub-agent panel (#1369 slice 4).
//!
//! Drives the REAL render path through the headless harness and the REAL wire
//! deserializer (`event_line`), asserting the hybrid rendering rule: one agent
//! in an environment → flat row with a dim `CN` badge; two or more agents →
//! one selectable environment row with agents nested below.

use super::tui_harness::*;
use crate::protocol::client::Event;
use crate::shell::keys::Key;

fn env_agent_json(id: &str, env_ref: &str) -> serde_json::Value {
    serde_json::json!({
        "agentId": id,
        "displayName": id,
        "agentUuid": format!("uuid-{id}"),
        "status": "running",
        "pid": 7,
        "readOnly": false,
        "executionBackend": "script",
        "environment": {
            "ref": env_ref,
            "name": "pr-env",
            "status": "running",
            "repository": "https://example.com/acme/widget.git",
            "branch": "pr-42",
            "runtimeId": "rt-9001",
            "workspace": "/work/pr-42",
            "socketMode": "proxy",
        },
    })
}

fn local_agent_json(id: &str) -> serde_json::Value {
    serde_json::json!({
        "agentId": id,
        "displayName": id,
        "agentUuid": format!("uuid-{id}"),
        "status": "running",
        "pid": 7,
        "readOnly": false,
    })
}

fn state_changed_line(agents: Vec<serde_json::Value>) -> String {
    serde_json::json!({ "type": "subagent_state_changed", "subagents": agents }).to_string()
}

/// A real `get_subagents` response line, exercising the roster-refresh path
/// (`handle_get_subagents` → `presentation_payloads::subagents`) rather than
/// a second live event.
fn get_subagents_response_line(agents: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "success": true,
        "data": { "subagents": agents },
    })
    .to_string()
}

/// A row's text after the selection column and tree-stalk characters.
fn after_stalk(row: &str) -> &str {
    row.trim_start_matches(['▌', ' ', '│', '├', '└'])
}

/// Non-empty panel row lines, excluding the bottom key-hint line.
fn panel_rows(panel: &str) -> Vec<String> {
    panel
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty() && !l.contains("⇥ pane"))
        .collect()
}

#[tokio::test]
async fn panel_width_still_clamps_to_half_the_terminal() {
    // At a 64-column terminal, half is 32: the OLD fixed width (30) stays under
    // the clamp while the raised width (34) must be clamped down to exactly 32 —
    // so this assertion fails until the width is raised AND the clamp holds.
    let mut h = TuiHarness::sized(64, 40).await;
    h.event(Event::AgentStart);
    let (panel_width, _, _) = h.app_mut().frame_split();
    assert_eq!(
        panel_width, 32,
        "the raised fixed width must still clamp to half a 64-column terminal"
    );
    // Boundary pinning around the crossover: at 68 columns half equals the
    // fixed width (34); at 70 the fixed width must win over half (35).
    for (cols, expected) in [(68usize, 34usize), (70, 34)] {
        let mut h = TuiHarness::sized(cols, 40).await;
        h.event(Event::AgentStart);
        let (panel_width, _, _) = h.app_mut().frame_split();
        assert_eq!(
            panel_width, expected,
            "panel width at a {cols}-column terminal must be {expected}"
        );
    }
}

#[tokio::test]
async fn solo_environment_agent_renders_flat_row_with_badge_between_stalk_and_name() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));

    let panel = h.left_panel();
    let rows = panel_rows(&panel);
    let row = rows
        .iter()
        .find(|l| l.contains("impl"))
        .unwrap_or_else(|| panic!("no panel row for impl:\n{panel}"));
    let stalk = row
        .find('└')
        .or_else(|| row.find('├'))
        .unwrap_or_else(|| panic!("solo row must keep its tree stalk:\n{row}"));
    let badge = row
        .find("C1")
        .unwrap_or_else(|| panic!("solo row must carry the C1 badge:\n{panel}"));
    let name = row.find("impl").expect("row contains the agent name");
    assert!(
        stalk < badge && badge < name,
        "the C1 badge must sit between the tree stalk and the name:\n{row}"
    );
    // Flat rendering: no separate environment row for a solo environment.
    assert_eq!(
        rows.iter().filter(|l| l.contains("C1")).count(),
        1,
        "a solo environment must occupy exactly one row:\n{panel}"
    );
}

#[tokio::test]
async fn solo_environment_badge_is_dim_styled() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));

    let raw = h.full_frame_raw();
    let dim_badge = crate::components::theme::dim("C1");
    assert!(
        raw.contains(&dim_badge),
        "the C1 badge must render in the dim style, raw frame:\n{raw}"
    );
}

#[tokio::test]
async fn solo_badge_and_name_truncate_at_narrow_widths() {
    let mut h = TuiHarness::sized(48, 40).await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json(
        "implementer-having-a-very-long-name",
        "C1",
    )]));

    let panel = h.left_panel();
    let row = panel_rows(&panel)
        .into_iter()
        .find(|l| l.contains("C1"))
        .unwrap_or_else(|| panic!("narrow row must keep the C1 badge:\n{panel}"));
    assert!(
        row.contains('…'),
        "the long agent name must truncate with an ellipsis inside the clamped panel:\n{row}"
    );
    assert!(
        unicode_width::UnicodeWidthStr::width(row.as_str()) <= 24,
        "the badged row must fit the 24-column clamped panel:\n{row}"
    );
}

#[tokio::test]
async fn shared_environment_groups_agents_under_one_selectable_environment_row() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![
        env_agent_json("impl", "C2"),
        env_agent_json("rev", "C2"),
    ]));

    let panel = h.left_panel();
    let rows = panel_rows(&panel);
    let env_rows: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("C2") && !l.contains("impl") && !l.contains("rev"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        env_rows.len(),
        1,
        "a shared environment must render exactly one environment row:\n{panel}"
    );
    let env_idx = env_rows[0];
    // Existing connector convention: non-last nested member draws `├ `, the
    // last draws `└ ` — not merely "some connector".
    for (id, connector) in [("impl", '├'), ("rev", '└')] {
        let idx = rows
            .iter()
            .position(|l| l.contains(id))
            .unwrap_or_else(|| panic!("no row for member {id}:\n{panel}"));
        assert!(
            idx > env_idx,
            "member {id} must nest beneath the environment row:\n{panel}"
        );
        assert!(
            rows[idx].contains(connector),
            "member {id} must carry the {connector} nested tree connector:\n{}",
            rows[idx]
        );
        assert_eq!(
            rows.iter().filter(|l| l.contains(id)).count(),
            1,
            "member {id} must not also appear as a duplicate root row:\n{panel}"
        );
    }
}

#[tokio::test]
async fn selecting_environment_row_renders_details_in_main_pane_chrome() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![
        env_agent_json("impl", "C2"),
        env_agent_json("rev", "C2"),
    ]));

    // Navigate to the environment row through the real panel keyboard path.
    let rows = panel_rows(&h.left_panel());
    let target = rows
        .iter()
        .position(|l| l.contains("C2") && !l.contains("impl") && !l.contains("rev"))
        .unwrap_or_else(|| panic!("no selectable environment row for C2:\n{}", rows.join("\n")));
    h.press(Key::Tab);
    for _ in 0..target {
        h.press(Key::Down);
    }
    h.press(Key::Enter);

    let top = h.main_pane();
    // Labeled probes so an agent-status string elsewhere in the pane cannot
    // satisfy the status/socket assertions vacuously.
    for needle in [
        "C2",
        "pr-env",
        "status: running",
        "https://example.com/acme/widget.git",
        "branch: pr-42",
        "rt-9001",
        "/work/pr-42",
        "socket: proxy",
    ] {
        assert!(
            top.contains(needle),
            "environment chrome must include {needle:?}, got:\n{top}"
        );
    }
}

#[tokio::test]
async fn environment_metadata_survives_sparse_snapshot_refresh() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    // Live event with full environment metadata …
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));
    // … then a REAL sparse `get_subagents` response omitting it entirely.
    h.event_line(&get_subagents_response_line(vec![local_agent_json("impl")]));

    let panel = h.left_panel();
    let row = panel_rows(&panel)
        .into_iter()
        .find(|l| l.contains("impl"))
        .unwrap_or_else(|| panic!("no panel row for impl:\n{panel}"));
    assert!(
        row.contains("C1"),
        "sticky merge must preserve the environment badge through a sparse refresh:\n{panel}"
    );
}

#[tokio::test]
async fn environment_details_survive_sparse_snapshot_refresh() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![
        env_agent_json("impl", "C2"),
        env_agent_json("rev", "C2"),
    ]));
    // A REAL sparse `get_subagents` roster refresh drops every environment
    // field; sticky merge must keep the DETAILS, not just the ref/badge.
    h.event_line(&get_subagents_response_line(vec![
        local_agent_json("impl"),
        local_agent_json("rev"),
    ]));

    let rows = panel_rows(&h.left_panel());
    let target = rows
        .iter()
        .position(|l| l.contains("C2") && !l.contains("impl") && !l.contains("rev"))
        .unwrap_or_else(|| {
            panic!(
                "the environment row must survive the sparse refresh:\n{}",
                rows.join("\n")
            )
        });
    h.press(Key::Tab);
    for _ in 0..target {
        h.press(Key::Down);
    }
    h.press(Key::Enter);

    let top = h.main_pane();
    for needle in [
        "C2",
        "pr-env",
        "status: running",
        "https://example.com/acme/widget.git",
        "branch: pr-42",
        "rt-9001",
        "/work/pr-42",
        "socket: proxy",
    ] {
        assert!(
            top.contains(needle),
            "environment details must survive the sparse refresh, missing {needle:?}:\n{top}"
        );
    }
}

#[tokio::test]
async fn local_only_session_renders_without_environment_chrome() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![local_agent_json("solo")]));

    let panel = h.left_panel();
    let rows = panel_rows(&panel);
    assert!(
        rows.iter().any(|l| l.contains("solo")),
        "local agent must render:\n{panel}"
    );
    assert!(
        !rows
            .iter()
            .any(|l| l.split_whitespace().any(|w| w.len() >= 2
                && w.starts_with('C')
                && w[1..].chars().all(|c| c.is_ascii_digit()))),
        "a local-only session must render no CN environment badges or rows:\n{panel}"
    );
    // Format-agnostic shape check: the local row must lead with the agent name
    // directly after the tree stalk — no badge token however styled/delimited.
    let solo_row = rows
        .iter()
        .find(|l| l.contains("solo"))
        .expect("local agent row");
    assert!(
        after_stalk(solo_row).starts_with("solo"),
        "local agent row must place the name directly after the stalk:\n{solo_row}"
    );
    // The only intended change for local-only sessions is the fixed width: every
    // panel line renders exactly 34 columns at a 120-wide terminal.
    let widths: std::collections::HashSet<usize> =
        panel.lines().map(|l| l.chars().count()).collect();
    assert_eq!(
        widths,
        std::collections::HashSet::from([34]),
        "panel lines must be exactly 34 columns wide:\n{panel}"
    );
}
