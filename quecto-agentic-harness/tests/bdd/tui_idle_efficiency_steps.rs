//! Step definitions for `tui_idle_efficiency.feature` (#978).
//!
//! These steps exercise production TUI scheduling predicates and git/Kitty
//! helpers through focused assertions. The full async event loop is covered by
//! quecto-tui unit tests so the BDD scenarios remain behavioural and stable.

use crate::QuectoWorld;
use cucumber::{given, then, when};
use quecto_tui::interface::app::tui_harness::{TuiHarness, subagents_changed};
use std::time::Duration;
use tempfile::TempDir;

fn with_harness<R>(world: &mut QuectoWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
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
fn given_no_visible_animation(world: &mut QuectoWorld) {
    with_harness(world, |_| {});
}

#[given("no notification is active")]
fn given_no_notification(world: &mut QuectoWorld) {
    with_harness(world, |_| {});
}

#[given("no subagent is active")]
fn given_no_active_subagent(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        h.event(subagents_changed(Vec::new()));
    });
}

#[given("no response is streaming")]
fn given_no_streaming(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        h.end_agent_run();
        h.set_streaming(false);
    });
}

#[when("the session is left idle")]
fn when_left_idle(world: &mut QuectoWorld) {
    with_harness(world, |_| {});
}

#[then("the TUI performs no sub-second periodic work")]
fn then_no_subsecond_work(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        assert!(
            !h.needs_animation_tick(false),
            "quiet idle sessions must not arm the sub-second animation timer"
        );
    });
}

#[given("the activity spinner is visible")]
fn given_activity_spinner_visible(world: &mut QuectoWorld) {
    let frame = with_harness(world, |h| {
        h.show_activity_spinner("working");
        h.spinner_frame_index().expect("visible spinner")
    });
    world.stdout = frame.to_string();
}

#[then("the activity spinner progresses")]
fn then_spinner_progresses(world: &mut QuectoWorld) {
    let before = world.stdout.parse::<usize>().expect("spinner frame");
    with_harness(world, |h| {
        let mut fallback_done = true;
        h.service_animation_tick(&mut fallback_done, tokio::time::Instant::now());
        assert_ne!(
            h.spinner_frame_index().expect("visible spinner"),
            before,
            "activity spinner should advance to a new frame"
        );
    });
}

#[given("a notification is visible")]
fn given_notification_visible(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        h.notify("Saved");
        assert!(h.has_notification(), "notification should start visible");
    });
}

#[then("the notification remains serviced until it is no longer visible")]
fn then_notification_is_serviced(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        assert!(
            !h.needs_animation_tick(false),
            "static notifications should not require the sub-second animation timer"
        );
        assert!(
            h.has_notification(),
            "fresh notification should remain visible"
        );
    });
}

#[given("the branch indicator shows the current branch")]
fn given_branch_indicator_current(world: &mut QuectoWorld) {
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
    world.stdout = "main".to_string();
}

#[when("the repository switches to another branch")]
fn when_repository_switches_branch(world: &mut QuectoWorld) {
    let repo = world
        ._extra_temp_dirs
        .last()
        .expect("branch repo temp dir")
        .path()
        .to_path_buf();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feature/branch\n")
        .expect("HEAD switch");
    world.stderr = "feature/branch".to_string();
}

#[then("the branch indicator shows the new branch within a few seconds")]
fn then_branch_updates_promptly(world: &mut QuectoWorld) {
    let branch = world.stderr.clone();
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
    assert_eq!(
        quecto_tui::interface::app::GIT_BRANCH_POLL_INTERVAL,
        Duration::from_secs(2),
        "branch refresh should avoid per-second polling but remain within a few seconds"
    );
}

#[given("the terminal does not confirm Kitty keyboard protocol support")]
fn given_terminal_without_kitty(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        h.clear_kitty_support();
    });
}

#[when("the fallback detection deadline passes")]
fn when_fallback_deadline_passes(world: &mut QuectoWorld) {
    with_harness(world, |_| {});
}

#[then("the TUI enables keyboard fallback mode")]
fn then_keyboard_fallback_enabled(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        let mut fallback_done = false;
        let deadline = tokio::time::Instant::now();
        h.service_animation_tick(&mut fallback_done, deadline);
        assert!(fallback_done, "fallback detection should complete");
        assert!(
            h.modify_other_keys_enabled(),
            "unsupported terminals should receive modifyOtherKeys fallback"
        );
    });
}

#[then("normal keyboard input is accepted")]
fn then_normal_keyboard_input_is_accepted(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        h.type_char('a');
        assert_eq!(
            h.editor_text(),
            "a",
            "normal key input should reach the editor"
        );
    });
}
