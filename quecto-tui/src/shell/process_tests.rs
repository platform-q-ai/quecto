use super::*;

// --- checked_pid tests ---

#[test]
fn checked_pid_normal_value() {
    assert_eq!(checked_pid(1234), Ok(1234));
}

#[test]
fn checked_pid_one() {
    assert_eq!(checked_pid(1), Ok(1));
}

#[test]
fn checked_pid_i32_max() {
    let max = i32::MAX as u32; // 2_147_483_647
    assert_eq!(checked_pid(max), Ok(i32::MAX));
}

#[test]
fn checked_pid_i32_max_plus_one_is_err() {
    let overflow = (i32::MAX as u32) + 1; // 2_147_483_648
    assert_eq!(checked_pid(overflow), Err(QuectodError::Overflow(overflow)));
}

#[test]
fn checked_pid_u32_max_is_err() {
    assert_eq!(checked_pid(u32::MAX), Err(QuectodError::Overflow(u32::MAX)));
}

#[test]
fn checked_pid_u32_max_does_not_produce_negative_one() {
    // This is the critical safety check: u32::MAX as i32 == -1,
    // and kill(-(-1), sig) == kill(1, sig) == kill init.
    let result = checked_pid(u32::MAX);
    assert!(result.is_err());
    // Verify the old unchecked cast WOULD have produced -1:
    assert_eq!(u32::MAX as i32, -1, "confirms the wrapping cast danger");
}

#[test]
fn checked_pid_zero_is_err() {
    assert_eq!(checked_pid(0), Err(QuectodError::Zero));
}

#[test]
fn pid_error_display_overflow() {
    let e = QuectodError::Overflow(2_147_483_648);
    let msg = e.to_string();
    assert!(msg.contains("2147483648"), "should mention the PID value");
    assert!(msg.contains("i32::MAX"), "should mention the limit");
}

#[test]
fn pid_error_display_zero() {
    let e = QuectodError::Zero;
    let msg = e.to_string();
    assert!(msg.contains("PID 0"), "should mention PID 0");
}

#[test]
fn pid_error_implements_std_error() {
    let e: Box<dyn std::error::Error> = Box::new(QuectodError::Zero);
    assert!(e.to_string().contains("PID 0"));
}

// --- kill_process_group tests ---

#[test]
fn kill_process_group_with_invalid_pid_returns_error() {
    // PID 999_999_999 almost certainly doesn't exist.
    let result = kill_process_group(999_999_999, libc::SIGTERM);
    // libc::kill returns -1 on error (ESRCH — no such process).
    assert_eq!(result, -1);
}

#[test]
fn kill_process_group_rejects_zero_pid() {
    // PID 0 would signal the caller's own process group.
    let result = kill_process_group(0, libc::SIGTERM);
    assert_eq!(result, -1);
}

#[test]
fn kill_process_group_rejects_negative_pid() {
    // Negative PID would flip sign and target an unrelated process.
    let result = kill_process_group(-1, libc::SIGTERM);
    assert_eq!(result, -1);
}

#[test]
fn kill_process_group_with_nonexistent_pid_returns_error() {
    // Use a PID that almost certainly doesn't exist as a process group.
    // Signal 0 is a null signal — only checks permissions.
    let result = kill_process_group(999_999_998, 0);
    // Should fail with ESRCH (no such process group).
    assert_eq!(result, -1);
}

#[test]
fn terminate_grace_ms_constant_is_reasonable() {
    const _: () = {
        assert!(TERMINATE_GRACE_MS >= 100);
        assert!(TERMINATE_GRACE_MS <= 5000);
    };
}

// --- grace window: long enough for the harness's SIGTERM teardown, but not
// paid in full when the tree is already gone ---

async fn spawn_group(script: &str) -> tokio::process::Child {
    tokio::process::Command::new("sh")
        .args(["-c", script])
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn test child")
}

#[tokio::test]
async fn grace_covers_a_child_that_cleans_up_on_sigterm() {
    // The child needs 600ms after SIGTERM before it can exit cleanly — the
    // shape of a harness running a container's kill script. It records
    // whether it finished or was cut off by SIGKILL.
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("finished");
    let script = format!(
        "trap 'sleep 0.6; : > {}; exit 0' TERM; while :; do sleep 0.05; done",
        marker.display()
    );
    let mut child = spawn_group(&script).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let started = std::time::Instant::now();
    assert!(terminate_child(&mut child, TERMINATE_GRACE_MS).await);
    assert!(
        marker.exists(),
        "a child that handles SIGTERM must be allowed to finish its teardown \
         before SIGKILL (took {:?}, grace {TERMINATE_GRACE_MS}ms)",
        started.elapsed()
    );
}

#[tokio::test]
async fn grace_ends_early_once_the_tree_has_exited() {
    // `sleep` dies on SIGTERM immediately; the terminate must not wait out
    // the whole grace window for an already-empty tree.
    let mut child = spawn_group("exec sleep 30").await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let started = std::time::Instant::now();
    assert!(terminate_child(&mut child, TERMINATE_GRACE_MS).await);
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(TERMINATE_GRACE_MS / 2),
        "terminate waited {elapsed:?} for a tree that exited immediately"
    );
}

#[tokio::test]
async fn the_old_200ms_window_cut_a_sigterm_teardown_off() {
    // Regression pin for the value that orphaned container environments: the
    // harness's kill script was SIGKILLed mid-run. Same child as above under
    // a 200ms window must NOT get to finish — proving the window is what the
    // passing test above depends on.
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("finished");
    let script = format!(
        "trap 'sleep 0.6; : > {}; exit 0' TERM; while :; do sleep 0.05; done",
        marker.display()
    );
    let mut child = spawn_group(&script).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    terminate_child(&mut child, 200).await;
    assert!(
        !marker.exists(),
        "a 200ms window cannot cover a 600ms teardown"
    );
}
