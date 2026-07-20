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
