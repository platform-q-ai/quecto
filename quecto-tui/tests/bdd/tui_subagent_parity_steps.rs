//! Step definitions for `tui_subagent_session_parity.feature` (#805).
//!
//! These drive the REAL TUI render/key path through the headless render harness
//! (`quecto_tui::shell::app::tui_harness`), exposed to this integration
//! target via the `test-harness` feature. Each step exercises observable
//! behaviour — the active session, the rendered frame, or the commands the
//! client would emit — rather than internal mechanics.

use super::*;
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::{
    self, TuiHarness, spawn_start, spawn_subagent_socket, spawn_subagent_socket_with_commands,
    subagent_with_socket, subagents_changed,
};
use quecto_tui::shell::keys::Key;

// ── Harness construction / driving helpers ──────────────────────────────────

/// Build a harness with `n` tracked sub-agents named `a1..aN`, each with a live,
/// drained socket so connect-on-commit succeeds (mirrors the unit-test harness).
async fn build_harness(n: usize) -> TuiHarness {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let mut infos = Vec::new();
    for i in 1..=n {
        let id = format!("a{i}");
        h.event(spawn_start(&id));
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

/// Create the runtime-backed harness in the world with `n` sub-agents.
fn init_harness(world: &mut TuiWorld, n: usize) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(build_harness(n));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

/// Run a closure against the harness within the runtime context (its key/select
/// paths spawn background tasks, so they need a live runtime).
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

/// Seed a session's OWN footer gauges (model + context usage [+ cost]) by
/// routing the forwarded events the real footer path consumes.
fn seed_subagent_footer(h: &mut TuiHarness, id: &str, model: &str, used: u64, cost: f64) {
    h.route(
        id,
        Event::Response {
            id: None,
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({ "model": model, "maxContextTokens": 200_000 })),
            error: None,
        },
    );
    h.route(
        id,
        Event::TurnEnd {
            message: serde_json::json!({ "contextTokens": used, "maxContextTokens": 200_000 }),
        },
    );
    h.route(
        id,
        Event::Response {
            id: None,
            command: "get_session_stats".into(),
            success: true,
            data: Some(serde_json::json!({
                "cost": cost,
                "contextTokens": used,
                "maxContextTokens": 200_000,
            })),
            error: None,
        },
    );
}

/// Seed the MASTER's own footer gauges (model + context usage).
fn seed_master_footer(h: &mut TuiHarness, model: &str, used: u64) {
    h.event(Event::Response {
        id: None,
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({ "model": model, "maxContextTokens": 200_000 })),
        error: None,
    });
    h.event(Event::TurnEnd {
        message: serde_json::json!({ "contextTokens": used, "maxContextTokens": 200_000 }),
    });
}

// ── Given ───────────────────────────────────────────────────────────────────

#[given(expr = "a TUI tracking sub-agent {string}")]
fn given_tracking(world: &mut TuiWorld, _id: String) {
    init_harness(world, 1);
}

#[given(expr = "a TUI tracking sub-agent {string} with its own workflow")]
fn given_tracking_with_workflow(world: &mut TuiWorld, id: String) {
    init_harness(world, 1);
    drive(world, |h| {
        h.route(&id, tui_harness::forwarded_workflow(&id, 2, 5));
    });
}

#[given(expr = "a TUI tracking sub-agent {string} with its own model and context usage")]
fn given_tracking_with_gauges(world: &mut TuiWorld, id: String) {
    init_harness(world, 1);
    drive(world, |h| {
        seed_master_footer(h, "mastrmdl", 100_000);
        seed_subagent_footer(h, &id, "subbymdl", 50_000, 0.0777);
    });
}

#[given(expr = "a TUI tracking sub-agent {string} with an open autocomplete popup")]
fn given_tracking_with_popup(world: &mut TuiWorld, _id: String) {
    init_harness(world, 1);
    drive(world, |h| {
        h.press(Key::Char('/'));
    });
}

#[given(expr = "a TUI tracking sub-agent {string} with focus on the panel")]
fn given_tracking_focus_panel(world: &mut TuiWorld, _id: String) {
    init_harness(world, 1);
    drive(world, |h| {
        h.press(Key::Tab);
    });
}

#[given("a TUI tracking two sub-agents with focus on the panel")]
fn given_tracking_two_focus_panel(world: &mut TuiWorld) {
    init_harness(world, 2);
    drive(world, |h| {
        h.press(Key::Tab);
    });
}

#[given(expr = "a TUI viewing sub-agent {string}")]
fn given_viewing(world: &mut TuiWorld, id: String) {
    given_viewing_with_status(world, id, "idle");
}

#[given(expr = "a TUI viewing running sub-agent {string}")]
fn given_viewing_running(world: &mut TuiWorld, id: String) {
    given_viewing_with_status(world, id, "running");
}

fn given_viewing_with_status(world: &mut TuiWorld, id: String, status: &str) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (mut h, cmd_rx) = rt.block_on(async {
        let mut h = TuiHarness::new().await;
        h.event(Event::AgentStart);
        h.event(spawn_start(&id));
        let (socket, cmd_rx) = spawn_subagent_socket_with_commands(&id);
        h.event(subagents_changed(vec![subagent_with_socket(
            &id,
            status,
            Some(("active", 0, 3)),
            Some(socket),
        )]));
        h.select(Some(&id));
        (h, cmd_rx)
    });
    h.try_drain_commands();
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_subagent_commands = Some(cmd_rx);
    world.tui_viewed_agent = Some(id);
}

#[given(expr = "a TUI viewing sub-agent {string} with focus on the panel")]
fn given_viewing_focus_panel(world: &mut TuiWorld, id: String) {
    init_harness(world, 1);
    drive(world, |h| {
        h.select(Some(&id));
        h.press(Key::Tab);
    });
}

#[given(expr = "I have selected sub-agent {string}")]
fn given_have_selected(world: &mut TuiWorld, id: String) {
    drive(world, |h| {
        h.select(Some(&id));
    });
}

// ── When ─────────────────────────────────────────────────────────────────────

#[when(expr = "I select sub-agent {string}")]
fn when_select(world: &mut TuiWorld, id: String) {
    drive(world, |h| {
        h.select(Some(&id));
    });
}

#[when("I return to the master")]
fn when_return_master(world: &mut TuiWorld) {
    drive(world, |h| {
        h.select(None);
    });
}

#[when("I press Tab")]
fn when_press_tab(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Tab);
    });
}

#[when("I press Tab again")]
fn when_press_tab_again(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Tab);
    });
}

#[when("I move the highlight down")]
fn when_move_down(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Down);
    });
}

#[when(expr = "I press digit {string}")]
fn when_press_digit(world: &mut TuiWorld, digit: String) {
    let c = digit.chars().next().expect("a digit");
    drive(world, |h| {
        h.press(Key::Char(c));
    });
}

#[when("I press Enter")]
fn when_press_enter(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Enter);
    });
}

#[when("I press Esc")]
fn when_press_esc(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Escape);
    });
}

#[when(expr = "I send the prompt {string}")]
fn when_send_prompt(world: &mut TuiWorld, prompt: String) {
    drive(world, |h| {
        h.submit(&prompt);
    });
}

#[when("I abort")]
fn when_abort(world: &mut TuiWorld) {
    drive(world, |h| {
        h.abort();
    });
}

// ── Then ─────────────────────────────────────────────────────────────────────

#[then(expr = "the active session is {string}")]
fn then_active_is(world: &mut TuiWorld, id: String) {
    let active = drive(world, |h| h.active_agent());
    assert_eq!(
        active.as_deref(),
        Some(id.as_str()),
        "active session mismatch"
    );
}

#[then("the active session is the master")]
fn then_active_is_master(world: &mut TuiWorld) {
    let active = drive(world, |h| h.active_agent());
    assert_eq!(active, None, "expected the master to be active");
}

#[then("the active session is unchanged")]
fn then_active_unchanged(world: &mut TuiWorld) {
    let active = drive(world, |h| h.active_agent());
    assert_eq!(
        active, None,
        "the active session must not change on movement"
    );
}

#[then(expr = "the view shows sub-agent {string}'s own workflow")]
fn then_view_shows_workflow(world: &mut TuiWorld, _id: String) {
    let frame = drive(world, |h| h.full_frame());
    // Sub-agent-first (#820): the boxed main-pane bar shows the agent's OWN
    // active issue (`#7`) on its title line — the issue title is dropped.
    assert!(
        frame.contains("#7"),
        "the active sub-agent's own workflow (active issue) must show, got:\n{frame}"
    );
}

#[then("the view no longer shows the sub-agent's workflow")]
fn then_view_hides_workflow(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        !frame.contains("#7"),
        "the master view must not show the sub-agent's workflow, got:\n{frame}"
    );
}

#[then("the footer shows the sub-agent's own model and context usage")]
fn then_footer_subagent(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains("subbymdl") && frame.contains("50k"),
        "the sub-agent's own footer gauges must show, got:\n{frame}"
    );
    assert!(
        !frame.contains("mastrmdl") && !frame.contains("100k"),
        "the master's footer gauges must NOT show while a sub-agent is active, got:\n{frame}"
    );
}

#[then("the footer shows the master's own model and context usage")]
fn then_footer_master(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains("mastrmdl") && frame.contains("100k"),
        "the master's own footer gauges must be restored, got:\n{frame}"
    );
    assert!(
        !frame.contains("subbymdl") && !frame.contains("50k"),
        "the sub-agent's footer gauges must NOT show while master is active, got:\n{frame}"
    );
}

#[then("focus is on the panel")]
fn then_focus_panel(world: &mut TuiWorld) {
    assert!(drive(world, |h| h.focus_on_panel()), "expected panel focus");
}

#[then("focus stays on the panel")]
fn then_focus_stays_panel(world: &mut TuiWorld) {
    assert!(
        drive(world, |h| h.focus_on_panel()),
        "focus must stay on the panel"
    );
}

#[then("focus is on the input")]
fn then_focus_input(world: &mut TuiWorld) {
    assert!(
        !drive(world, |h| h.focus_on_panel()),
        "expected input focus"
    );
}

#[then("focus stays on the input")]
fn then_focus_stays_input(world: &mut TuiWorld) {
    assert!(
        !drive(world, |h| h.focus_on_panel()),
        "focus must stay on the input"
    );
}

#[then(expr = "the prompt appears in sub-agent {string}'s session")]
fn then_prompt_in_session(world: &mut TuiWorld, _id: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains("message for child"),
        "the prompt must land in the active sub-agent's session body, got:\n{frame}"
    );
}

#[then("no prompt is sent to the master")]
fn then_no_master_prompt(world: &mut TuiWorld) {
    let cmds = drain_master_commands(world);
    assert!(
        !cmds.iter().any(|c| {
            (c.contains("\"type\":\"prompt\"") || c.contains("\"type\":\"follow_up\""))
                && (c.contains("message for child") || c.contains("message for running child"))
        }),
        "the prompt must route to the active sub-agent, not the master: {cmds:?}"
    );
}

#[then(expr = "the follow-up is sent to sub-agent {string}")]
fn then_follow_up_sent_to_subagent(world: &mut TuiWorld, _id: String) {
    let cmds = drain_subagent_commands(world);
    assert!(
        cmds.iter().any(|c| {
            c.contains("\"type\":\"follow_up\"") && c.contains("message for running child")
        }),
        "running sub-agent submit must emit a follow-up to the child: {cmds:?}"
    );
    world.tui_last_commands = cmds;
}

#[then("the sub-agent command does not claim steer")]
fn then_subagent_command_not_steer(world: &mut TuiWorld) {
    assert!(
        !world.tui_last_commands.iter().any(|c| {
            c.contains("\"type\":\"steer\"") || c.contains("\"streamingBehavior\":\"steer\"")
        }),
        "running sub-agent submit must not claim steer behavior: {:?}",
        world.tui_last_commands
    );
}

#[then("no abort is sent to the master")]
fn then_no_master_abort(world: &mut TuiWorld) {
    let cmds = drain_master_commands(world);
    assert!(
        !cmds.iter().any(|c| c.contains("\"abort\"")),
        "abort must target the active sub-agent, not the master: {cmds:?}"
    );
}

#[then("a vertical divider is drawn between the panel and the body")]
fn then_divider_drawn(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains('│'),
        "expected a vertical divider between panel and body, got:\n{frame}"
    );
}

#[then("the divider styling reflects the focused pane")]
fn then_divider_styling(world: &mut TuiWorld) {
    // After Tab focuses the panel, the divider's styling must differ from the
    // input-focused frame. Compare the raw (ANSI-bearing) frame before/after.
    let panel_focus = drive(world, |h| {
        let after = h.full_frame_raw();
        // Toggle back to input focus to get the contrasting styling.
        h.press(Key::Tab);
        let input_focus = h.full_frame_raw();
        (after, input_focus)
    });
    assert_ne!(
        panel_focus.0, panel_focus.1,
        "the divider styling must differ between panel-focused and input-focused"
    );
}

// ── #828 Part 1: full conversation backfill on select ───────────────────────

#[given(expr = "sub-agent {string} has streamed the live token {string} since selection")]
fn given_streamed_live(world: &mut TuiWorld, id: String, token: String) {
    drive(world, |h| {
        h.route(&id, Event::Token { token });
    });
}

#[when(expr = "the backfill history {string} then {string} arrives")]
fn when_backfill_arrives(world: &mut TuiWorld, user: String, assistant: String) {
    // Route to the agent captured by the "viewing sub-agent" given, not a
    // literal id, so this step is reusable for any agent (#828 review).
    let id = world
        .tui_viewed_agent
        .clone()
        .expect("a 'viewing sub-agent' given must run first to capture the id");
    // Mirror the kernel's get_messages payload the connect-on-select backfill
    // requests: a user/assistant transcript that pre-dates the live stream.
    let data = serde_json::json!({
        "messages": [
            { "role": "user", "content": user },
            { "role": "assistant", "content": assistant },
        ]
    });
    drive(world, |h| {
        h.route(
            &id,
            Event::Response {
                id: Some("subagent-history".into()),
                command: "get_messages".into(),
                success: true,
                data: Some(data),
                error: None,
            },
        );
    });
}

#[then(expr = "the sub-agent's session shows {string}")]
fn then_session_shows(world: &mut TuiWorld, text: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&text),
        "the selected sub-agent's session must show {text:?}, got:\n{frame}"
    );
}

#[then(expr = "the sub-agent's session still shows {string}")]
fn then_session_still_shows(world: &mut TuiWorld, text: String) {
    // Same check as `then_session_shows`; the distinct prose ("still") documents
    // that the backfill PRESERVED earlier live content (#828 review). Inlined so
    // the assertion is visible in this step body.
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&text),
        "the backfill must preserve live content {text:?}, got:\n{frame}"
    );
}

#[then(expr = "{string} appears above {string} in the session")]
fn then_appears_above(world: &mut TuiWorld, upper: String, lower: String) {
    let frame = drive(world, |h| h.full_frame());
    let up = frame.find(&upper);
    let lo = frame.find(&lower);
    assert!(
        matches!((up, lo), (Some(u), Some(l)) if u < l),
        "history {upper:?} must render ABOVE live content {lower:?}, got:\n{frame}"
    );
}

fn drain_subagent_commands(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let rx = world
        .tui_subagent_commands
        .as_mut()
        .expect("sub-agent command receiver");
    handle.block_on(async {
        let mut cmds = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            cmds.push(cmd);
        }
        cmds
    })
}

/// Drain the commands the MASTER client would have emitted (within the runtime).
fn drain_master_commands(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    handle.block_on(h.drain_commands())
}

// ── A sub-agent footer's own cache-hit ratio ────────────────────────────────

/// The child's harness totals: 42 uncached input, 530,402 cache-read and
/// 43,394 cache-write tokens, a normalized hit ratio of 92.4%.
const CHILD_CACHE_READ: u64 = 530_402;
const CHILD_CACHE_WRITE: u64 = 43_394;
const CHILD_UNCACHED_INPUT: u64 = 42;

/// Wait for the commands the child socket received to settle, then return them.
fn settle_subagent_commands(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let rx = world
        .tui_subagent_commands
        .as_mut()
        .expect("sub-agent command receiver");
    handle.block_on(tui_harness::drain_child_commands_until_quiet(rx))
}

fn stats_request_count(commands: &[String]) -> usize {
    commands
        .iter()
        .filter(|c| c.contains("\"type\":\"get_session_stats\""))
        .count()
}

/// The child's own `get_session_stats` reply, as its harness serializes it:
/// before its run (no usage recorded yet) or after it.
fn child_stats_reply(after_run: bool) -> Event {
    let data = match after_run {
        true => serde_json::json!({
            "tokens": {
                "input": CHILD_UNCACHED_INPUT,
                "cacheRead": CHILD_CACHE_READ,
                "cacheWrite": CHILD_CACHE_WRITE,
            },
            "cacheHitRatio": CHILD_CACHE_READ as f64
                / (CHILD_UNCACHED_INPUT + CHILD_CACHE_READ + CHILD_CACHE_WRITE) as f64,
            "contextTokens": 43_396,
            "maxContextTokens": 300_000,
        }),
        false => serde_json::json!({
            "tokens": { "input": 0, "cacheRead": 0, "cacheWrite": 0 },
            "cacheHitRatio": null,
        }),
    };
    Event::Response {
        id: Some("subagent-stats".into()),
        command: "get_session_stats".into(),
        success: true,
        data: Some(data),
        error: None,
    }
}

#[when(expr = "sub-agent {string} ends a turn")]
fn when_subagent_ends_turn(world: &mut TuiWorld, id: String) {
    // The connect-time stats request is not the turn's: wait until the child
    // has received it, and answer it as the child would before its run ends.
    let mut connect = Vec::new();
    for _ in 0..50 {
        connect.extend(settle_subagent_commands(world));
        if stats_request_count(&connect) > 0 {
            break;
        }
    }
    assert_eq!(
        stats_request_count(&connect),
        1,
        "the feed's connect asks the child for its stats once: {connect:?}"
    );
    drive(world, |h| {
        h.route(&id, child_stats_reply(false));
        h.route(
            &id,
            Event::TurnEnd {
                message: serde_json::json!({
                    "contextTokens": 43_396,
                    "maxContextTokens": 300_000,
                }),
            },
        );
    });
}

#[then(expr = "the TUI asks sub-agent {string} for its own session stats")]
fn then_asks_subagent_stats(world: &mut TuiWorld, id: String) {
    // The captured commands are the socket of the sub-agent the Given views.
    assert_eq!(
        world.tui_viewed_agent.as_deref(),
        Some(id.as_str()),
        "the scenario must name the viewed sub-agent"
    );
    let cmds = settle_subagent_commands(world);
    assert_eq!(
        stats_request_count(&cmds),
        1,
        "a sub-agent's turn end must ask that child for its stats once: {cmds:?}"
    );
}

#[when(expr = "sub-agent {string} answers with its session stats")]
fn when_subagent_answers_stats(world: &mut TuiWorld, id: String) {
    drive(world, |h| {
        h.route(&id, child_stats_reply(true));
    });
}

#[then("the footer shows the sub-agent's own cache-hit ratio")]
fn then_footer_subagent_cache_hit(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains("hit 92.4%") && frame.contains("cache 530k/43k"),
        "the sub-agent footer must show its own normalized cache figures, got:\n{frame}"
    );
}
