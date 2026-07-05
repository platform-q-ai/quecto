//! Steps for `tui_pid_safety.feature`.
//!
//! Exercises the REAL checked PID conversion used by the TUI's process-group
//! cleanup: `quecto_tui::infrastructure::process::checked_pid`. Production
//! signals a process group with `libc::kill(-pid, sig)`, so the group target is
//! the *negated* checked pid — we assert that relationship rather than sending a
//! real signal. `kill_process_group` is `pub(crate)` and cannot be called from
//! this external crate; the live SIGTERM→grace→SIGKILL sequence is `@pending`.

use super::*;
use quecto_tui::infrastructure::process::checked_pid;

// ── Background ────────────────────────────────────────────────────────────

#[given("the TUI spawns an agent as a child process")]
fn given_spawns_child(world: &mut QuectoWorld) {
    // Fresh conversion state for the scenario.
    world.tui_pid_input = None;
    world.tui_pid_result = None;
    world.tui_pid_group_target = None;
}

#[given("the child process runs in its own process group")]
fn given_own_group(world: &mut QuectoWorld) {
    // The child is spawned in its own group (setsid/setpgid in production); the
    // conversion under test turns its PID into the negated group signal target.
    world.tui_pid_group_target = None;
}

// ── PID under test ────────────────────────────────────────────────────────

#[given(regex = r"^the child process has PID (\d+)$")]
fn given_child_pid(world: &mut QuectoWorld, pid: u32) {
    world.tui_pid_input = Some(pid);
}

// ── Conversion ────────────────────────────────────────────────────────────

#[when("the TUI converts the PID for process group kill")]
fn when_convert(world: &mut QuectoWorld) {
    let pid = world.tui_pid_input.expect("PID under test");
    match checked_pid(pid) {
        Ok(checked) => {
            world.tui_pid_result = Some(Ok(checked));
            // Production targets the group via `libc::kill(-pid, sig)`.
            world.tui_pid_group_target = Some(-checked);
        }
        Err(e) => {
            world.tui_pid_result = Some(Err(e.to_string()));
            world.tui_pid_group_target = None;
        }
    }
}

// ── Assertions ────────────────────────────────────────────────────────────

#[then(regex = r"^the converted PID should be (\d+)$")]
fn then_converted_pid(world: &mut QuectoWorld, expected: i32) {
    let result = world.tui_pid_result.as_ref().expect("conversion result");
    assert_eq!(
        result.as_ref().ok(),
        Some(&expected),
        "checked_pid should convert to {expected}, got {result:?}"
    );
}

#[then(regex = r"^SIGTERM should be sent to process group (-\d+)$")]
fn then_group_target(world: &mut QuectoWorld, group: i32) {
    assert_eq!(
        world.tui_pid_group_target,
        Some(group),
        "the process-group signal target must be the negated checked pid"
    );
}

#[then("the conversion should fail")]
fn then_conversion_fails(world: &mut QuectoWorld) {
    let result = world.tui_pid_result.as_ref().expect("conversion result");
    assert!(
        result.is_err(),
        "conversion should be rejected, got {result:?}"
    );
}

#[then("no signal should be sent")]
fn then_no_signal(world: &mut QuectoWorld) {
    assert!(
        world.tui_pid_result.as_ref().is_some_and(Result::is_err),
        "a rejected conversion must not yield a signal target"
    );
    assert_eq!(
        world.tui_pid_group_target, None,
        "no process-group target should be produced for a rejected PID"
    );
}

#[then("the error should mention the PID value")]
fn then_error_mentions_pid(world: &mut QuectoWorld) {
    let pid = world.tui_pid_input.expect("PID under test");
    let msg = match world.tui_pid_result.as_ref().expect("conversion result") {
        Err(e) => e.clone(),
        Ok(v) => panic!("expected an error mentioning the PID, got Ok({v})"),
    };
    assert!(
        msg.contains(&pid.to_string()),
        "error should mention the offending PID {pid}, got: {msg}"
    );
}

#[then("SIGTERM must NOT be sent to PID 1")]
fn then_not_pid_1(world: &mut QuectoWorld) {
    // The naive `u32::MAX as i32` wraps to -1, whose group target `-(-1)` is 1
    // (init). The checked path rejects it instead of ever producing that.
    let pid = world.tui_pid_input.expect("PID under test");
    assert_eq!(pid as i32, -1, "u32::MAX casts to -1 without the guard");
    assert!(
        world.tui_pid_result.as_ref().is_some_and(Result::is_err),
        "u32::MAX must be rejected so no signal ever reaches PID 1"
    );
    assert_ne!(
        world.tui_pid_group_target,
        Some(1),
        "the group target must never be PID 1 (init)"
    );
}
