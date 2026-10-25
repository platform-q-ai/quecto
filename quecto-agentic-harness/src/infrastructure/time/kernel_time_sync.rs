//! The host's word on its clock (epic #2705), for the board server's
//! clock gate.
//!
//! On Linux (glibc) it is the kernel's NTP state, read by `ntp_adjtime`
//! with no mode bits set: a read, which changes nothing and needs no
//! privilege. The clock is synchronised only when the kernel's state is
//! one it keeps while synchronised ([`KEPT`], a leap second pending or
//! passing included) and `STA_UNSYNC` is clear; anything else (a failed
//! read, `TIME_ERROR`, `STA_UNSYNC`, a state this build does not know) is
//! not. Elsewhere no state is read, so the clock is never affirmed
//! synchronised and a board server there takes no change that reads it.
use crate::application::project_board::ports::{ClockSync, TimeSync};

/// The kernel's word on its clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct KernelTimeSync;

impl TimeSync for KernelTimeSync {
    fn sync(&self) -> ClockSync {
        read()
    }
}

/// The states the kernel keeps while its clock is synchronised: the
/// allowlist [`judged`] lets through.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const KEPT: [libc::c_int; 5] = [
    libc::TIME_OK,
    libc::TIME_INS,
    libc::TIME_DEL,
    libc::TIME_OOP,
    libc::TIME_WAIT,
];

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn read() -> ClockSync {
    // SAFETY: `timex` is a plain C struct of integers, for which all zero bytes are a valid value.
    let mut state: libc::timex = unsafe { std::mem::zeroed() };
    assert_eq!(state.modes, 0, "a read sets no mode bits");
    // SAFETY: `state` is a live, writable `timex` for the whole call; with no mode bits the kernel only fills it.
    let clock = unsafe { libc::ntp_adjtime(&mut state) };
    match clock {
        -1 => ClockSync::Unsynced(format!(
            "the kernel's clock state could not be read: {}",
            std::io::Error::last_os_error()
        )),
        _ => judged(clock, state.status),
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn read() -> ClockSync {
    ClockSync::Unsynced("this build reads the clock's synchronisation on Linux only".into())
}

/// The kernel's `clock` state and `status` bits, judged: synchronised only
/// for a state in [`KEPT`] with `STA_UNSYNC` clear.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn judged(clock: libc::c_int, status: libc::c_int) -> ClockSync {
    let kept = KEPT.contains(&clock);
    // The kernel's own flag for "not synchronised": synchronised is its
    // absence, together with a kept state.
    let unsync_clear = status & libc::STA_UNSYNC == 0;
    match (kept, unsync_clear) {
        (true, true) => ClockSync::Synced,
        (true, false) => ClockSync::Unsynced(
            "the kernel marks the clock unsynchronised (STA_UNSYNC): no time daemon keeps it in step"
                .into(),
        ),
        (false, _) if clock == libc::TIME_ERROR => ClockSync::Unsynced(
            "the kernel reports a clock error (TIME_ERROR): the clock is not synchronised".into(),
        ),
        (false, _) => ClockSync::Unsynced(format!(
            "the kernel reports clock state {clock}, which this build does not take as synchronised"
        )),
    }
}

#[cfg(test)]
#[path = "kernel_time_sync_tests.rs"]
mod tests;
