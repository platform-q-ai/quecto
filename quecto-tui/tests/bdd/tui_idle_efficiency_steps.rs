//! Step definitions for `tui_idle_efficiency.feature` (#978).
//!
//! These steps exercise production TUI scheduling predicates and git/Kitty
//! helpers through focused assertions. The full async event loop is covered by
//! quecto-tui unit tests so the BDD scenarios remain behavioural and stable.

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::shell::app::tui_harness::{TuiHarness, subagents_changed};
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

#[given("the TUI has no visible animation")]
fn given_no_visible_animation(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            h.spinner_frame_index().is_none(),
            "no activity spinner should be visible"
        );
    });
}

#[given("no notification is active")]
fn given_no_notification(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(!h.has_notification(), "no notification should be visible");
    });
}

#[given("no subagent is active")]
fn given_no_active_subagent(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.event(subagents_changed(Vec::new()));
    });
}

#[given("no response is streaming")]
fn given_no_streaming(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.end_agent_run();
        h.set_streaming(false);
    });
}

#[when("the session is left idle")]
fn when_left_idle(world: &mut TuiWorld) {
    // Being "left idle" means the loop only performs its periodic servicing
    // pass, with no user input or agent events — drive exactly that pass.
    with_harness(world, |h| {
        let mut fallback_done = true;
        h.service_animation_tick(&mut fallback_done, tokio::time::Instant::now());
    });
}

#[then("the TUI performs no sub-second periodic work")]
fn then_no_subsecond_work(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            !h.needs_animation_tick(false),
            "quiet idle sessions must not arm the sub-second animation timer"
        );
    });
}

#[given("the activity spinner is visible")]
fn given_activity_spinner_visible(world: &mut TuiWorld) {
    let frame = with_harness(world, |h| {
        h.show_activity_spinner("working");
        h.spinner_frame_index().expect("visible spinner")
    });
    world.tui_idle_spinner_frame = Some(frame);
}

#[then("the activity spinner progresses")]
fn then_spinner_progresses(world: &mut TuiWorld) {
    let before = world
        .tui_idle_spinner_frame
        .expect("spinner frame captured by the Given step");
    with_harness(world, |h| {
        assert_ne!(
            h.spinner_frame_index().expect("visible spinner"),
            before,
            "activity spinner should advance to a new frame"
        );
    });
}

#[given("a notification is visible")]
fn given_notification_visible(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.notify("Saved");
        assert!(h.has_notification(), "notification should start visible");
    });
}

#[then("the notification remains serviced until it is no longer visible")]
fn then_notification_is_serviced(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            !h.needs_animation_tick(false),
            "static notifications should not require the sub-second animation timer"
        );
        assert!(
            h.has_notification(),
            "unexpired notification should remain visible after the idle pass"
        );
    });
}

#[given("the branch indicator shows the current branch")]
fn given_branch_indicator_current(world: &mut TuiWorld) {
    let tmp = TempDir::new().expect("branch test temp dir");
    let repo = tmp.path().to_path_buf();
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").expect("HEAD");
    with_harness(world, |h| {
        h.set_git_repo(repo);
        assert!(h.apply_branch(Some("main".to_string())));
        assert!(h.bottom_stack().contains("main"));
    });
    world._extra_temp_dirs.push(tmp);
}

#[when("the repository switches to another branch")]
fn when_repository_switches_branch(world: &mut TuiWorld) {
    let repo = world
        ._extra_temp_dirs
        .last()
        .expect("branch repo temp dir")
        .path()
        .to_path_buf();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feature/branch\n")
        .expect("HEAD switch");
    world.tui_idle_expected_branch = Some("feature/branch".to_string());
}

#[then("the branch indicator shows the new branch within a few seconds")]
fn then_branch_updates_promptly(world: &mut TuiWorld) {
    let branch = world
        .tui_idle_expected_branch
        .clone()
        .expect("expected branch recorded by the When step");
    // The next periodic poll (bounded to a few seconds by unit tests on
    // GIT_BRANCH_POLL_INTERVAL) runs this same production refresh task.
    let changed = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .block_on(async {
            world
                .tui_parity
                .as_mut()
                .expect("TUI harness")
                .0
                .refresh_branch_from_repo()
                .await
        });
    assert!(changed, "branch refresh should observe the switched branch");
    with_harness(world, |h| {
        assert!(
            h.bottom_stack().contains(&branch),
            "branch indicator should show the switched branch"
        );
    });
}

#[given("the terminal does not confirm Kitty keyboard protocol support")]
fn given_terminal_without_kitty(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.clear_kitty_support();
    });
    world.tui_idle_fallback_done = Some(false);
}

#[when("the fallback detection deadline passes")]
fn when_fallback_deadline_passes(world: &mut TuiWorld) {
    let mut fallback_done = world
        .tui_idle_fallback_done
        .expect("fallback state initialised by the Given step");
    with_harness(world, |h| {
        // An already-elapsed deadline is exactly "the deadline has passed";
        // the service pass is what the event loop runs when it fires.
        let deadline = tokio::time::Instant::now();
        h.service_animation_tick(&mut fallback_done, deadline);
    });
    world.tui_idle_fallback_done = Some(fallback_done);
}

#[then("the TUI enables keyboard fallback mode")]
fn then_keyboard_fallback_enabled(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_idle_fallback_done,
        Some(true),
        "fallback detection should complete once the deadline passes"
    );
    with_harness(world, |h| {
        assert!(
            h.modify_other_keys_enabled(),
            "unsupported terminals should receive modifyOtherKeys fallback"
        );
    });
}

#[then("normal keyboard input is accepted")]
fn then_normal_keyboard_input_is_accepted(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.type_char('a');
        assert_eq!(
            h.editor_text(),
            "a",
            "normal key input should reach the editor"
        );
    });
}
