//! Step definitions for `tui_multi_tab_fix_pass.feature` (#1466 fix pass,
//! PR #1485 field regressions).
//!
//! Reuses the polish-suite harness plumbing (`a TUI with a second background
//! tab`, `the background tab becomes the active tab`, `the resume selector
//! opens with workspaces` live in `tui_multi_tab_polish_steps.rs`). Clicks
//! target the RECORDED tab-bar hit ranges, never guessed columns.

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::components::ansi::strip_ansi;
use quecto_tui::shell::app::tui_harness::TuiHarness;
use tempfile::TempDir;

fn with_harness<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { TuiHarness::new().await });
        world.tui_parity = Some(crate::TuiParityHarness(h));
    }
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

fn manifest_path(world: &mut TuiWorld) -> std::path::PathBuf {
    if world._extra_temp_dirs.is_empty() {
        world
            ._extra_temp_dirs
            .push(TempDir::new().expect("tempdir"));
    }
    world._extra_temp_dirs[0].path().join("manifests.json")
}

// ── Item 1: tab bar blocks ───────────────────────────────────────────────

#[when("the tab bar renders")]
fn when_tab_bar_renders(world: &mut TuiWorld) {
    world.stdout = with_harness(world, |h| h.tab_bar_line(80));
}

#[then("the bar shows the active tab as a cyan block and the rest as dim blocks")]
fn then_bar_number_blocks(world: &mut TuiWorld) {
    // Roles are falsifiable: the CYAN reverse-video block must be the ACTIVE
    // tab (1), the dim reverse-video block the background tab (2) — swapping
    // the styles must fail here.
    assert!(
        world.stdout.contains("\x1b[7;36m 1 "),
        "the active tab (1) must be the reverse-video cyan block; bar={:?}",
        world.stdout
    );
    assert!(
        world.stdout.contains("\x1b[7;2m 2 "),
        "the background tab (2) must be a reverse-video dim block; bar={:?}",
        world.stdout
    );
}

#[then("the bar never shows a default \":Master\" suffix")]
fn then_bar_no_master_suffix(world: &mut TuiWorld) {
    assert!(
        !world.stdout.contains(":Master"),
        "unnamed tabs render the bare 1-based number, never ':Master'; bar={:?}",
        world.stdout
    );
}

#[then("the bar ends with a dim new-tab button")]
fn then_bar_plus_button(world: &mut TuiWorld) {
    let stripped = strip_ansi(&world.stdout);
    assert!(
        stripped.trim_end().ends_with('+'),
        "the bar must END with the ' + ' new-tab button; bar={stripped:?}"
    );
    assert!(
        world.stdout.ends_with("\x1b[2m + \x1b[0m"),
        "the ' + ' button must render dim; bar={:?}",
        world.stdout
    );
}

#[when("the user clicks inside the second tab block")]
fn when_click_second_block(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.click_tab_block(2);
    });
}

#[when("the user clicks the new-tab button")]
fn when_click_plus(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.click_new_tab_button();
    });
}

#[then("a third tab is open")]
fn then_third_tab(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(
            h.tab_count(),
            3,
            "clicking the trailing ' + ' button must open a new tab"
        );
    });
}

#[when("the user clicks past the end of the tab bar")]
fn when_click_past_bar(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.click_past_tab_bar();
    });
}

#[then("the first tab is still the active tab")]
fn then_first_tab_still_active(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(
            h.active_tab_index(),
            0,
            "a click in the bar's dead space must not switch tabs"
        );
    });
}

#[then("no new tab is open")]
fn then_no_new_tab(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(
            h.tab_count(),
            2,
            "a click in the bar's dead space must not open a tab"
        );
    });
}

// ── Item 2: terminal-safe cycle chords (three tabs — direction matters) ──

#[given("a TUI with two background tabs")]
fn given_two_background_tabs(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.open_background_tab();
        h.open_second_background_tab();
    });
}

#[when("the user presses Ctrl+PageDown")]
fn when_ctrl_pgdn(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press_raw(b"\x1b[6;5~");
    });
}

#[when("the user presses Ctrl+PageUp")]
fn when_ctrl_pgup(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press_raw(b"\x1b[5;5~");
    });
}

#[then("the second tab becomes the active tab")]
fn then_second_tab_active(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(
            h.active_tab_index(),
            1,
            "Ctrl+PageDown must cycle FORWARD to the next tab"
        );
    });
}

#[then("the last tab becomes the active tab")]
fn then_last_tab_active(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(
            h.active_tab_index(),
            2,
            "Ctrl+PageUp from the first tab must wrap BACK to the last tab"
        );
    });
}

// ── Item 3: resume overlay recency + recognizable rows ───────────────────

#[given("two durable workspaces with different last-active times")]
fn given_two_workspaces(world: &mut TuiWorld) {
    let path = manifest_path(world);
    TuiHarness::seed_workspace_row(&path, "ws-old", "Old Spike", 1_000, Some("s-old"), None);
    TuiHarness::seed_workspace_row(&path, "ws-new", "New Spike", 2_000, Some("s-new"), None);
}

#[then("the first workspace row is the most recently active one")]
fn then_first_row_most_recent(world: &mut TuiWorld) {
    let first = world.stdout.lines().next().unwrap_or_default();
    assert!(
        first.contains("New Spike"),
        "workspaces must sort by last-active descending; rows:\n{}",
        world.stdout
    );
}

#[given("a durable workspace last active two hours ago")]
fn given_workspace_two_hours_old(world: &mut TuiWorld) {
    let path = manifest_path(world);
    let two_hours_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs()
        - 2 * 3_600
        - 30;
    TuiHarness::seed_workspace_row(&path, "ws-aged", "Aged WS", two_hours_ago, Some("s0"), None);
}

#[then(expr = "the workspace row shows {string}")]
fn then_row_shows(world: &mut TuiWorld, text: String) {
    assert!(
        world.stdout.contains(&text),
        "workspace rows must show a relative last-active time; rows:\n{}",
        world.stdout
    );
}

#[given(expr = "a durable workspace whose tab summary is {string}")]
fn given_workspace_with_summary(world: &mut TuiWorld, summary: String) {
    let path = manifest_path(world);
    TuiHarness::seed_workspace_row(
        &path,
        "ws-snip",
        "Snippet WS",
        1_755_000_000,
        Some("s0"),
        Some(&summary),
    );
}

#[then(expr = "the workspace row shows the snippet {string}")]
fn then_row_shows_snippet(world: &mut TuiWorld, snippet: String) {
    assert!(
        world.stdout.contains(&snippet),
        "workspace rows must show a per-tab conversation snippet, not just \
         label + tab count; rows:\n{}",
        world.stdout
    );
}

// ── Item 4: workspace resurrection with stale stored tab ids ─────────────

#[when("a workspace manifest with stale stored tab ids is restored")]
fn when_stale_manifest_restored(world: &mut TuiWorld) {
    let (a, b) = with_harness(world, |h| h.apply_manifest_with_stale_tab_ids());
    world.stdout = format!("sess-a:{a} sess-b:{b}");
}

#[then("every stored session is carried by a tab")]
fn then_every_session_carried(world: &mut TuiWorld) {
    assert_eq!(
        world.stdout, "sess-a:true sess-b:true",
        "the FIRST manifest entry (reused master slot) must resume its \
         session like every other tab"
    );
}

// ── Item 5: dead sub-agents must not swallow messages ────────────────────

#[given("a detached sub-agent is focused")]
fn given_detached_subagent_focused(world: &mut TuiWorld) {
    // `select` may spawn feed tasks; enter the runtime for the duration.
    with_harness(world, |_| ());
    let rt = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let _guard = rt.enter();
    let h = &mut world.tui_parity.as_mut().expect("TUI harness").0;
    h.track_subagent("w1", "detached");
    h.select(Some("w1"));
}

#[when("the user submits a message to the focused sub-agent")]
fn when_submit_to_subagent(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.submit("hello there");
    });
}

#[then("a delivery failure naming the sub-agent is visibly surfaced")]
fn then_delivery_outcome_surfaced(world: &mut TuiWorld) {
    with_harness(world, |h| {
        // The surfaced text must SPECIFICALLY reference the failed delivery
        // and the agent — an incidental unrelated notification cannot pass.
        let status = h.last_status_line().unwrap_or_default();
        let last_note = h.last_notification().unwrap_or_default();
        let surfaced = |s: &str| s.contains("not delivered") && s.contains("w1");
        assert!(
            surfaced(&status) || surfaced(&last_note),
            "a message to a dead/unattached sub-agent must surface a delivery \
             failure naming the agent; last status={status:?}, last \
             notification={last_note:?}"
        );
    });
}

// ── Item 6: background spinners keep animating ───────────────────────────

#[given("a running turn on the background tab")]
fn given_running_background_turn(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.sourced_stream_event_for_tab(1, quecto_tui::protocol::client::Event::AgentStart);
        assert!(
            h.tab_spinner(1),
            "precondition: the background turn must light the tab spinner"
        );
    });
}

#[when("an animation service tick runs")]
fn when_animation_tick_runs(world: &mut TuiWorld) {
    let (bar_before, repaint) = with_harness(world, |h| {
        let before = h.tab_bar_line(80);
        let mut kitty_done = true;
        let repaint = h.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());
        (before, repaint)
    });
    world.stdout = bar_before;
    world.tui_idle_fallback_done = Some(repaint);
}

#[then("the rendered tab-bar spinner glyph changes")]
fn then_bar_spinner_advances(world: &mut TuiWorld) {
    let before = world.stdout.clone();
    with_harness(world, |h| {
        let after = h.tab_bar_line(80);
        assert_ne!(
            after, before,
            "the animation tick must visibly advance the BACKGROUND tab's \
             spinner glyph in the rendered tab bar"
        );
    });
}

#[then("the animation tick requests a repaint")]
fn then_tick_repaints(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_idle_fallback_done,
        Some(true),
        "a busy background tab must schedule the bar-cadence repaint"
    );
}
