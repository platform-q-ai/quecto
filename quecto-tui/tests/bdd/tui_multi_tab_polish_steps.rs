//! Step definitions for `tui_multi_tab_polish.feature` (#1466).
//!
//! Background-tab paint gating, spinner/unread-dot semantics, per-tab
//! retained-session cap, workspace label/last-active resume + orphan GC,
//! and the kitty Ctrl+Tab alias — all through the headless harness.

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::protocol::client::Event;
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

fn stream_background_tokens(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| {
        for i in 0..n {
            h.sourced_stream_event_for_tab(
                1,
                Event::Token {
                    token: format!("bg-token-{i} "),
                },
            );
        }
    });
}

fn manifest_path(world: &mut TuiWorld) -> std::path::PathBuf {
    if world._extra_temp_dirs.is_empty() {
        world
            ._extra_temp_dirs
            .push(TempDir::new().expect("tempdir"));
    }
    world._extra_temp_dirs[0].path().join("manifests.json")
}

#[given("a TUI with a second background tab")]
fn given_second_background_tab(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.open_background_tab();
        assert_eq!(h.active_tab_index(), 0, "focus must stay on the first tab");
    });
}

#[when(expr = "{int} tokens stream to the background tab")]
fn when_tokens_stream_background(world: &mut TuiWorld, n: usize) {
    let frames = with_harness(world, |h| h.rendered_frames());
    world.tui_render_count = Some(frames);
    stream_background_tokens(world, n);
}

#[given(expr = "{int} tokens already streamed to the background tab")]
fn given_tokens_already_streamed(world: &mut TuiWorld, n: usize) {
    stream_background_tokens(world, n);
    // Guard: the clear-on-switch Then below is only meaningful if the dot is
    // actually set first (anti-tautology, RED evidence lives here).
    with_harness(world, |h| {
        assert!(
            h.tab_unread(1),
            "precondition: streamed background output must set the unread dot"
        );
    });
}

#[then("no frame is painted for the background stream")]
fn then_no_frame_painted(world: &mut TuiWorld) {
    let before = world.tui_render_count.expect("baseline recorded");
    with_harness(world, |h| {
        assert_eq!(
            h.rendered_frames(),
            before,
            "background-tab tokens must not paint frames (#1466 decision 3)"
        );
    });
}

#[then("no frame is painted even after the render loop settles")]
fn then_no_frame_after_settle(world: &mut TuiWorld) {
    let before = world.tui_render_count.expect("baseline recorded");
    with_harness(world, |h| {
        // Drain any deferred/coalesced paint the loop would run at its
        // deadline; a background stream must not have scheduled one.
        h.settle_deferred_paints();
        assert_eq!(
            h.rendered_frames(),
            before,
            "background-tab tokens must not schedule a deferred paint / loop wakeup"
        );
    });
}

#[then("the background tab is marked unread")]
fn then_background_unread(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            h.tab_unread(1),
            "background output since last view must set the unread dot"
        );
    });
}

#[when("the user switches to the background tab")]
fn when_switch_to_background(world: &mut TuiWorld) {
    let frames = with_harness(world, |h| h.rendered_frames());
    world.tui_render_count = Some(frames);
    with_harness(world, |h| {
        h.switch_to_tab(1);
    });
}

#[then("exactly one frame is painted for the switch")]
fn then_one_frame_for_switch(world: &mut TuiWorld) {
    let before = world.tui_render_count.expect("baseline recorded");
    with_harness(world, |h| {
        assert_eq!(
            h.rendered_frames(),
            before + 1,
            "a tab switch paints exactly one frame"
        );
    });
}

#[then("the background tab is no longer marked unread")]
fn then_background_read(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            !h.tab_unread(1),
            "switching to a tab must clear its unread dot"
        );
    });
}

fn start_background_turn(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.sourced_stream_event_for_tab(1, Event::AgentStart);
    });
}

#[given("the background tab has a running turn")]
fn given_background_running(world: &mut TuiWorld) {
    start_background_turn(world);
}

#[when("a turn starts on the background tab")]
fn when_background_turn_starts(world: &mut TuiWorld) {
    start_background_turn(world);
}

#[when("the background turn ends")]
fn when_background_turn_ends(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.end_background_turn(1);
    });
}

#[then("the background tab shows no spinner")]
fn then_background_no_spinner(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            !h.tab_spinner(1),
            "an ended/aborted background turn must clear the tab spinner"
        );
    });
}

#[then("the background tab shows a spinner")]
fn then_background_spinner(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            h.tab_spinner(1),
            "a running background turn must show the tab spinner"
        );
    });
}

#[then("the TUI still requests animation ticks while only a background tab is busy")]
fn then_animation_tick_any_tab(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            h.animation_tick_needed(),
            "needs_animation_tick must consider a busy background tab (#1466 scope 2)"
        );
    });
}

#[given(expr = "{int} sub-agent sessions already started on the background tab")]
fn given_background_sessions(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| {
        h.start_sessions_on_tab(1, n);
    });
}

#[when(expr = "{int} sub-agent sessions start on the active tab")]
fn when_sessions_start_active(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| {
        h.start_sessions_on_tab(0, n);
    });
}

#[then(expr = "the active tab retains exactly {int} sessions")]
fn then_active_retains(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| {
        assert_eq!(
            h.retained_sessions_for_tab(0),
            n,
            "retained-session cap must be 30 per tab (#1466 decision 2)"
        );
    });
}

#[then(expr = "the background tab still retains exactly {int} sessions")]
fn then_background_retains(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| {
        assert_eq!(
            h.retained_sessions_for_tab(1),
            n,
            "eviction on one tab must not touch another tab's retained sessions"
        );
    });
}

#[given(expr = "a durable workspace labelled {string}")]
fn given_labelled_workspace(world: &mut TuiWorld, label: String) {
    let path = manifest_path(world);
    TuiHarness::seed_workspace_manifest(
        &path,
        "3f2b6c1e-9d4a-4b6f-8c2d-5e7a1b9c0d42",
        &label,
        1_755_000_000,
        true,
    );
}

#[when("the resume selector opens with workspaces")]
fn when_resume_opens(world: &mut TuiWorld) {
    let path = manifest_path(world);
    let rows = with_harness(world, |h| h.open_resume_with_manifest(&path));
    world.stdout = rows
        .iter()
        .map(|(l, d)| format!("{l} | {d}"))
        .collect::<Vec<_>>()
        .join("\n");
}

#[then(expr = "the workspace row shows the label {string}")]
fn then_row_shows_label(world: &mut TuiWorld, label: String) {
    assert!(
        world.stdout.contains(&label),
        "resume rows must list workspaces by label (#1466 decision 1); rows:\n{}",
        world.stdout
    );
}

#[then("the workspace row does not show the raw workspace id")]
fn then_row_hides_raw_id(world: &mut TuiWorld) {
    assert!(
        !world
            .stdout
            .contains("3f2b6c1e-9d4a-4b6f-8c2d-5e7a1b9c0d42"),
        "resume rows must not surface the raw UUID; rows:\n{}",
        world.stdout
    );
}

#[given("a durable workspace with no resumable sessions")]
fn given_orphaned_workspace(world: &mut TuiWorld) {
    let path = manifest_path(world);
    TuiHarness::seed_workspace_manifest(&path, "orphan-ws-1", "Orphan", 1_755_000_000, false);
}

#[when("workspace garbage collection runs")]
fn when_gc_runs(world: &mut TuiWorld) {
    let path = manifest_path(world);
    // The removed-id report itself is covered by unit tests
    // (`gc_orphaned_removes_workspaces_with_no_sessions_and_no_registry_rows`);
    // this scenario asserts only the observable resume behaviour.
    let _removed = TuiHarness::gc_orphaned_workspaces(&path);
}

#[then("the orphaned workspace is not offered for resume")]
fn then_orphan_gone(world: &mut TuiWorld) {
    assert!(
        !world.stdout.contains("Orphan") && !world.stdout.contains("orphan-ws-1"),
        "an orphaned workspace must not appear in the resume selector; rows:\n{}",
        world.stdout
    );
}

#[when("the kitty Ctrl+Tab sequence is pressed")]
fn when_kitty_ctrl_tab(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press_raw(b"\x1b[9;5u");
    });
}

#[then("the background tab becomes the active tab")]
fn then_background_active(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(
            h.active_tab_index(),
            1,
            "kitty Ctrl+Tab must alias the Alt tab-cycle primary (#1466 decision 5)"
        );
    });
}
