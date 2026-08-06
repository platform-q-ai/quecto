//! Step definitions for `tui_environment_grouping.feature` (#1369 slice 4,
//! revised by the #1369 follow-up: solo environments render as full groups
//! and environment selection shows container information only).
//!
//! These drive the REAL TUI render path through the headless render harness
//! (`quecto_tui::shell::app::tui_harness`) and feed sub-agent rosters through
//! the REAL wire deserializer (`event_line`), so the versioned wire fields
//! (`executionBackend`, `environment`) are exercised end to end — including
//! the `get_subagents` response path for roster refreshes. Assertions read the
//! composed frame — no hard-coded output strings.

use super::*;
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

/// One wire subagent object carrying script-managed environment metadata.
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

/// One wire subagent object with no environment metadata at all.
fn sparse_agent_json(id: &str) -> serde_json::Value {
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
    serde_json::json!({
        "type": "subagent_state_changed",
        "subagents": agents,
    })
    .to_string()
}

/// A real `get_subagents` response line, exercising the roster-refresh path
/// (`handle_get_subagents` → `presentation_payloads::subagents`).
fn get_subagents_response_line(agents: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "success": true,
        "data": { "subagents": agents },
    })
    .to_string()
}

async fn build(width: usize, agents: Vec<serde_json::Value>) -> TuiHarness {
    let mut h = TuiHarness::sized(width, 40).await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(agents));
    h
}

fn init(world: &mut TuiWorld, width: usize, agents: Vec<serde_json::Value>) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(build(width, agents));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_env_terminal_cols = Some(width);
}

fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    f(h)
}

fn panel(world: &mut TuiWorld) -> String {
    drive(world, |h| h.left_panel())
}

// Panel-chrome helpers are shared from the harness so the footer-hint filter
// and stalk glyph set cannot drift from the render code.
use quecto_tui::shell::app::tui_harness::{after_stalk, label_depth, panel_rows};

/// Structural environment-row detection: the environment ref is the row's own
/// first label token (agent rows lead with the agent name; nested members of
/// a grouped environment carry no ref at all). Not tied to scenario-specific
/// agent names, so the steps stay reusable.
fn is_environment_row(row: &str, env_ref: &str) -> bool {
    let label = after_stalk(row);
    label.starts_with(env_ref)
        && label[env_ref.len()..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ')
}

// ── Given ────────────────────────────────────────────────────────────────────

#[given(
    expr = "a TUI on a {int}-column terminal tracking sub-agent {string} running alone in environment {string}"
)]
fn given_solo_env_agent(world: &mut TuiWorld, cols: usize, id: String, env_ref: String) {
    init(world, cols, vec![env_agent_json(&id, &env_ref)]);
}

#[given(
    expr = "a TUI on a {int}-column terminal tracking sub-agents {string} and {string} sharing environment {string}"
)]
fn given_shared_env_agents(
    world: &mut TuiWorld,
    cols: usize,
    a: String,
    b: String,
    env_ref: String,
) {
    init(
        world,
        cols,
        vec![env_agent_json(&a, &env_ref), env_agent_json(&b, &env_ref)],
    );
}

#[given(expr = "a TUI on a {int}-column terminal tracking local-only sub-agent {string}")]
fn given_local_only(world: &mut TuiWorld, cols: usize, id: String) {
    init(world, cols, vec![sparse_agent_json(&id)]);
    world.tui_env_local_agents = vec![id];
}

// ── When ─────────────────────────────────────────────────────────────────────

#[when(expr = "I select the environment row {string} through panel navigation")]
fn when_select_environment_row(world: &mut TuiWorld, env_ref: String) {
    let rows = panel_rows(&panel(world));
    let target = rows
        .iter()
        .position(|l| is_environment_row(l, &env_ref))
        .unwrap_or_else(|| panic!("no environment row for {env_ref}:\n{}", rows.join("\n")));
    drive(world, |h| {
        h.press(Key::Tab); // focus the panel (cursor starts on the master row)
        for _ in 0..target {
            h.press(Key::Down);
        }
        h.press(Key::Enter);
    });
}

#[when(expr = "a sparse get_subagents roster refresh omits the environment metadata for {string}")]
fn when_sparse_refresh(world: &mut TuiWorld, id: String) {
    // A REAL `get_subagents` response carrying neither executionBackend nor
    // environment — sticky merge must preserve the earlier live metadata
    // through the response-path parse, not just live events.
    let line = get_subagents_response_line(vec![sparse_agent_json(&id)]);
    drive(world, |h| {
        h.event_line(&line);
    });
}

#[when(
    expr = "a sparse get_subagents roster refresh omits the environment metadata for {string} and {string}"
)]
fn when_sparse_refresh_two(world: &mut TuiWorld, a: String, b: String) {
    let line = get_subagents_response_line(vec![sparse_agent_json(&a), sparse_agent_json(&b)]);
    drive(world, |h| {
        h.event_line(&line);
    });
}

// ── Then ─────────────────────────────────────────────────────────────────────

#[then(
    expr = "the agent {string} is nested beneath the {string} environment row with the last-child connector"
)]
fn then_solo_nested(world: &mut TuiWorld, id: String, env_ref: String) {
    let panel = panel(world);
    let rows = panel_rows(&panel);
    let env_idx = rows
        .iter()
        .position(|l| is_environment_row(l, &env_ref))
        .unwrap_or_else(|| panic!("no environment row for {env_ref}:\n{panel}"));
    let idx = rows
        .iter()
        .position(|l| l.contains(&id))
        .unwrap_or_else(|| panic!("no row for member {id}:\n{panel}"));
    assert!(
        idx > env_idx,
        "member {id} must be listed beneath the environment row:\n{panel}"
    );
    // Structural nesting, not mere ordering: the member's label must start
    // strictly deeper than the environment row's — a flat root-level sibling
    // row (same depth, later position) must fail this step.
    assert!(
        label_depth(&rows[idx]) > label_depth(&rows[env_idx]),
        "member {id} must nest strictly deeper than the environment row:\n{panel}"
    );
    assert!(
        rows[idx].contains('└'),
        "the solo member must carry the └ last-child connector:\n{}",
        rows[idx]
    );
}

#[then(expr = "the agent {string} appears exactly once, beneath the environment row")]
fn then_member_exactly_once_nested(world: &mut TuiWorld, id: String) {
    let panel = panel(world);
    let rows = panel_rows(&panel);
    let idx = rows
        .iter()
        .position(|l| l.contains(&id))
        .unwrap_or_else(|| panic!("no panel row for {id}:\n{panel}"));
    // The member row leads with the agent name directly after the tree stalk
    // and sits strictly deeper than the row above it (its environment row).
    assert!(
        after_stalk(&rows[idx]).starts_with(&id),
        "member row must place the name directly after the stalk:\n{}",
        rows[idx]
    );
    assert!(
        idx > 0 && label_depth(&rows[idx]) > label_depth(&rows[idx - 1]),
        "member {id} must sit nested beneath the row above it:\n{panel}"
    );
    assert_eq!(
        rows.iter().filter(|l| l.contains(&id)).count(),
        1,
        "agent {id} must appear exactly once:\n{panel}"
    );
}

#[then(expr = "the panel shows one environment row for {string}")]
fn then_env_row(world: &mut TuiWorld, env_ref: String) {
    let panel = panel(world);
    let env_rows: Vec<String> = panel_rows(&panel)
        .into_iter()
        .filter(|l| is_environment_row(l, &env_ref))
        .collect();
    assert_eq!(
        env_rows.len(),
        1,
        "a shared environment must render exactly one selectable environment row:\n{panel}"
    );
}

#[then(
    expr = "the agents {string} and {string} are nested beneath the {string} environment row with tree connectors"
)]
fn then_nested_agents(world: &mut TuiWorld, a: String, b: String, env_ref: String) {
    let panel = panel(world);
    let rows = panel_rows(&panel);
    let env_idx = rows
        .iter()
        .position(|l| is_environment_row(l, &env_ref))
        .unwrap_or_else(|| panic!("no environment row for {env_ref}:\n{panel}"));
    // Existing connector convention: the non-last nested member draws `├ `,
    // the last draws `└ ` — not just "some connector".
    for (id, connector) in [(&a, '├'), (&b, '└')] {
        let idx = rows
            .iter()
            .position(|l| l.contains(id.as_str()))
            .unwrap_or_else(|| panic!("no row for member {id}:\n{panel}"));
        assert!(
            idx > env_idx,
            "member {id} must be listed beneath the environment row:\n{panel}"
        );
        // Structural nesting, not mere ordering (see then_solo_nested).
        assert!(
            label_depth(&rows[idx]) > label_depth(&rows[env_idx]),
            "member {id} must nest strictly deeper than the environment row:\n{panel}"
        );
        let row = &rows[idx];
        assert!(
            row.contains(connector),
            "member {id} must carry the {connector} nested tree connector:\n{row}"
        );
    }
}

#[then(expr = "no duplicate root rows are rendered for {string} or {string}")]
fn then_no_duplicate_roots(world: &mut TuiWorld, a: String, b: String) {
    let panel = panel(world);
    for id in [&a, &b] {
        let count = panel_rows(&panel)
            .into_iter()
            .filter(|l| l.contains(id.as_str()))
            .count();
        assert_eq!(
            count, 1,
            "agent {id} must appear exactly once (nested), never duplicated at the root:\n{panel}"
        );
    }
}

#[then(
    expr = "the main pane shows environment details for {string} including name, status, repository, branch, runtime id, workspace and socket mode"
)]
fn then_env_details(world: &mut TuiWorld, env_ref: String) {
    let top = drive(world, |h| h.main_pane());
    for needle in [
        env_ref.as_str(),
        "pr-env",
        // Labeled probes so an agent-status string elsewhere in the pane
        // cannot satisfy the status/socket assertions vacuously.
        "status: running",
        "https://example.com/acme/widget.git",
        "branch: pr-42",
        "rt-9001",
        "/work/pr-42",
        "socket: proxy",
    ] {
        assert!(
            top.contains(needle),
            "selected environment chrome must include {needle:?}, got:\n{top}"
        );
    }
}

#[then("the panel contains no environment badge or environment row")]
fn then_no_env_chrome(world: &mut TuiWorld) {
    let panel = panel(world);
    let rows = panel_rows(&panel);
    assert!(
        !rows.iter().any(|l| l.split_whitespace().any(|w| {
            w.len() >= 2 && w.starts_with('C') && w[1..].chars().all(|c| c.is_ascii_digit())
        })),
        "a local-only session must render no CN environment badges:\n{panel}"
    );
    // Format-agnostic shape check: each local agent row must lead with the
    // agent name directly after the tree stalk — NO token (however styled or
    // delimited) may sit between the stalk and the name.
    for id in world.tui_env_local_agents.clone() {
        let row = rows
            .iter()
            .find(|l| l.contains(&id))
            .unwrap_or_else(|| panic!("no panel row for local agent {id}:\n{panel}"));
        assert!(
            after_stalk(row).starts_with(&id),
            "local agent row must place the name directly after the stalk:\n{row}"
        );
    }
}

#[then("the left panel is rendered 34 columns wide")]
fn then_panel_width(world: &mut TuiWorld) {
    let cols = world.tui_env_terminal_cols.expect("terminal size given");
    let panel = panel(world);
    let widths: std::collections::HashSet<usize> =
        panel.lines().map(|l| l.chars().count()).collect();
    assert_eq!(
        widths,
        std::collections::HashSet::from([34]),
        "every panel line must be exactly 34 columns at a {cols}-wide terminal:\n{panel}"
    );
}

#[given("a parent conversation is on screen")]
fn given_parent_conversation_on_screen(world: &mut TuiWorld) {
    // Composable atop any roster Given: seeds the master transcript through
    // the REAL token-stream + turn-end path, then asserts it renders.
    drive(world, |h| {
        h.event(Event::Token {
            token: "PARENT_CONVERSATION_MARKER".to_string(),
        });
        h.event_line(
            &serde_json::json!({
                "type": "turn_end",
                "message": {"role": "assistant", "content": "PARENT_CONVERSATION_MARKER"},
            })
            .to_string(),
        );
    });
    let top = drive(world, |h| h.main_pane());
    assert!(
        top.contains("PARENT_CONVERSATION_MARKER"),
        "precondition: the parent conversation renders before selection:\n{top}"
    );
}

#[then(
    expr = "the main pane carries a container-info header and lists the members {string} and {string}"
)]
fn then_container_info_pane(world: &mut TuiWorld, a: String, b: String) {
    let top = drive(world, |h| h.main_pane());
    assert!(
        top.contains("Container environment"),
        "the pane must carry a clear container-info header:\n{top}"
    );
    assert!(top.contains("members:"), "member roster renders:\n{top}");
    for member in [&a, &b] {
        assert!(
            top.contains(member.as_str()),
            "member {member} must be listed in the container info:\n{top}"
        );
    }
}

#[then("the main pane does not render the parent conversation")]
fn then_no_parent_conversation(world: &mut TuiWorld) {
    let top = drive(world, |h| h.main_pane());
    assert!(
        !top.contains("PARENT_CONVERSATION_MARKER"),
        "no parent transcript may render beneath the environment info:\n{top}"
    );
}

#[then("the agent name is truncated within the clamped panel width")]
fn then_narrow_truncation(world: &mut TuiWorld) {
    // Independent width bound computed from the scenario's stated terminal
    // size and the spec (fixed 34 clamped to half the terminal) — NOT from the
    // app's own split, so deleting the clamp fails this step.
    let cols = world.tui_env_terminal_cols.expect("terminal size given");
    let bound = usize::min(34, cols / 2);
    let panel = panel(world);
    let row = panel_rows(&panel)
        .into_iter()
        .find(|l| l.contains('…'))
        .unwrap_or_else(|| panic!("the long agent name must truncate with an ellipsis:\n{panel}"));
    assert!(
        unicode_width::UnicodeWidthStr::width(row.as_str()) <= bound,
        "the truncated row must fit the clamped panel width {bound}:\n{row}"
    );
}
