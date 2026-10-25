use super::*;

/// Reading the kernel's state answers either way, and never panics: the
/// host running the tests may or may not keep its clock synchronised.
#[test]
fn the_kernel_is_asked_without_failing() {
    match KernelTimeSync.sync() {
        ClockSync::Synced => {}
        ClockSync::Unsynced(why) => assert!(!why.is_empty(), "an unsynced clock says why"),
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod linux {
    use super::*;

    fn unsynced(clock: libc::c_int, status: libc::c_int) -> String {
        match judged(clock, status) {
            ClockSync::Unsynced(why) => why,
            ClockSync::Synced => panic!("state {clock}, status {status:#x}: unsynced"),
        }
    }

    /// The states the kernel keeps while synchronised, a leap second
    /// pending or passing included, with `STA_UNSYNC` clear.
    #[test]
    fn a_kept_state_with_the_unsync_bit_clear_is_synchronised() {
        for clock in [
            libc::TIME_OK,
            libc::TIME_INS,
            libc::TIME_DEL,
            libc::TIME_OOP,
            libc::TIME_WAIT,
        ] {
            assert_eq!(judged(clock, libc::STA_PLL), ClockSync::Synced, "{clock}");
        }
    }

    #[test]
    fn the_unsync_bit_makes_any_state_unsynchronised() {
        let why = unsynced(libc::TIME_OK, libc::STA_UNSYNC | libc::STA_PLL);
        assert!(why.contains("STA_UNSYNC"), "{why}");
    }

    #[test]
    fn a_clock_error_is_unsynchronised() {
        let why = unsynced(libc::TIME_ERROR, 0);
        assert!(why.contains("TIME_ERROR"), "{why}");
    }

    /// Anything not affirmed is unsynchronised: a failed read, and a state
    /// this build does not know.
    #[test]
    fn a_failed_read_or_an_unknown_state_is_unsynchronised() {
        assert!(unsynced(-1, 0).contains("-1"));
        assert!(unsynced(42, 0).contains("42"));
    }
}
