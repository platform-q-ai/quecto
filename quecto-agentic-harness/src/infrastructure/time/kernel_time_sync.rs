//! The host's word on its clock (epic #2705), for the board server's
//! clock gate.
//!
//! On Linux it is the kernel's NTP state, read by `adjtimex` (the system
//! call `ntp_adjtime` wraps; glibc and musl both have it) with no mode
//! bits set: a read, which changes nothing and needs no privilege. The
//! clock is synchronised only when `STA_UNSYNC` is clear and the kernel's
//! state is one it keeps while synchronised ([`KEPT`], a leap second
//! pending or passing included); anything else (a failed call,
//! `STA_UNSYNC`, `TIME_ERROR`, a state this build does not know) is not.
//! Elsewhere no state is read, so the clock is never affirmed synchronised
//! and a board server there takes no change that reads it.
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
/// allowlist [`classified`] lets through.
#[cfg(target_os = "linux")]
const KEPT: [libc::c_int; 5] = [
    libc::TIME_OK,
    libc::TIME_INS,
    libc::TIME_DEL,
    libc::TIME_OOP,
    libc::TIME_WAIT,
];

#[cfg(target_os = "linux")]
fn read() -> ClockSync {
    // SAFETY: `timex` is a plain C struct of integers, for which all zero bytes are a valid value.
    let mut state: libc::timex = unsafe { std::mem::zeroed() };
    assert_eq!(state.modes, 0, "a read sets no mode bits");
    // SAFETY: `state` is a live, writable `timex` for the whole call; with no mode bits the kernel only fills it.
    let ret = unsafe { libc::adjtimex(&mut state) };
    // Read at once, before anything else can set it; used only for -1.
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    classified(ret, state.status, errno)
}

#[cfg(not(target_os = "linux"))]
fn read() -> ClockSync {
    ClockSync::Unsynced("this build reads the clock's synchronisation on Linux only".into())
}

/// The call's return `ret` (the kernel's clock state, or -1 with `errno`)
/// and its `status` bits, judged: synchronised only with `STA_UNSYNC`
/// clear and a state in [`KEPT`]. The bit is read first: the kernel
/// answers `TIME_ERROR` while it is set, and the bit says why.
#[cfg(target_os = "linux")]
fn classified(ret: libc::c_int, status: libc::c_int, errno: i32) -> ClockSync {
    let unsync_set = status & libc::STA_UNSYNC != 0;
    match (ret, unsync_set) {
        (-1, _) => ClockSync::Unsynced(format!(
            "the kernel's clock state could not be read: {}",
            std::io::Error::from_raw_os_error(errno)
        )),
        (_, true) => ClockSync::Unsynced(
            "the kernel marks the clock unsynchronised (STA_UNSYNC): no time daemon has told it \
             the clock is in step (systemd-timesyncd, or chrony with rtcsync, does)"
                .into(),
        ),
        (clock, false) if KEPT.contains(&clock) => ClockSync::Synced,
        (libc::TIME_ERROR, false) => ClockSync::Unsynced(
            "the kernel reports a clock error (TIME_ERROR): the clock is not synchronised".into(),
        ),
        (clock, false) => ClockSync::Unsynced(format!(
            "the kernel reports clock state {clock}, which this build does not take as synchronised"
        )),
    }
}

#[cfg(test)]
#[path = "kernel_time_sync_tests.rs"]
mod tests;
