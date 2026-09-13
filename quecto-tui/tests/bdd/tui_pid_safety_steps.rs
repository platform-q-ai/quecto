//! Steps for `tui_pid_safety.feature`.
//!
//! Exercises the REAL checked PID conversion used by the TUI's leader-only
//! exit signal (#1956): `quecto_tui::shell::process::checked_pid`. Production
//! signals exactly one process with `libc::kill(pid, sig)` — never a group —
//! so the target is the checked pid itself; we assert that relationship
//! rather than sending a real signal. The live SIGTERM → budget → SIGKILL
//! sequence is covered by `tui_ctrl_d_exit.feature`.

use super::*;
use quecto_tui::shell::process::checked_pid;

// ── Background ────────────────────────────────────────────────────────────

#[given("the TUI spawns an agent as a child process")]
fn given_spawns_child(world: &mut TuiWorld) {
    // Fresh conversion state for the scenario.
    world.tui_pid_input = None;
    world.tui_pid_result = None;
    world.tui_pid_group_target = None;
}

#[given("the child process runs in its own process group")]
fn given_own_group(world: &mut TuiWorld) {
    // The child is spawned in its own group (setpgid in production) so the
    // post-exit canary can recognise a stray; the group is never signalled.
    world.tui_pid_group_target = None;
}

// ── PID under test ────────────────────────────────────────────────────────

#[given(regex = r"^the child process has PID (\d+)$")]
fn given_child_pid(world: &mut TuiWorld, pid: u32) {
    world.tui_pid_input = Some(pid);
}

// ── Conversion ────────────────────────────────────────────────────────────

#[when("the TUI converts the PID for the leader signal")]
fn when_convert(world: &mut TuiWorld) {
    let pid = world.tui_pid_input.expect("PID under test");
    match checked_pid(pid) {
        Ok(checked) => {
            world.tui_pid_result = Some(Ok(checked));
            // Production targets the one process via `libc::kill(pid, sig)`.
            world.tui_pid_group_target = Some(checked);
        }
        Err(e) => {
            world.tui_pid_result = Some(Err(e.to_string()));
            world.tui_pid_group_target = None;
        }
    }
}

// ── Assertions ────────────────────────────────────────────────────────────

#[then(regex = r"^the converted PID should be (\d+)$")]
fn then_converted_pid(world: &mut TuiWorld, expected: i32) {
    let result = world.tui_pid_result.as_ref().expect("conversion result");
    assert_eq!(
        result.as_ref().ok(),
        Some(&expected),
        "checked_pid should convert to {expected}, got {result:?}"
    );
}

#[then(regex = r"^SIGTERM should be sent to the single process (\d+)$")]
fn then_group_target(world: &mut TuiWorld, pid: i32) {
    assert_eq!(
        world.tui_pid_group_target,
        Some(pid),
        "the signal target must be the checked pid itself, never a group"
    );
    assert!(pid > 0, "a group target would be non-positive");
}

#[then("the conversion should fail")]
fn then_conversion_fails(world: &mut TuiWorld) {
    let result = world.tui_pid_result.as_ref().expect("conversion result");
    assert!(
        result.is_err(),
        "conversion should be rejected, got {result:?}"
    );
}

#[then("no signal should be sent")]
fn then_no_signal(world: &mut TuiWorld) {
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
fn then_error_mentions_pid(world: &mut TuiWorld) {
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
fn then_not_pid_1(world: &mut TuiWorld) {
    // The naive `u32::MAX as i32` wraps to -1: `kill(-1)` addresses every
    // process. The checked path rejects it instead of ever producing that.
    let pid = world.tui_pid_input.expect("PID under test");
    assert_eq!(pid as i32, -1, "u32::MAX casts to -1 without the guard");
    assert!(
        world.tui_pid_result.as_ref().is_some_and(Result::is_err),
        "u32::MAX must be rejected so no signal ever reaches PID 1"
    );
    assert_ne!(
        world.tui_pid_group_target,
        Some(1),
        "the target must never be PID 1 (init)"
    );
}
