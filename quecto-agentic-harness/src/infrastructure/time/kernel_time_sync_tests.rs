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

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    const NO_DAEMON: &str = "no time daemon keeps it in step";

    fn unsynced(ret: libc::c_int, status: libc::c_int, errno: i32) -> String {
        match classified(ret, status, errno) {
            ClockSync::Unsynced(why) => why,
            ClockSync::Synced => panic!("return {ret}, status {status:#x}: unsynced"),
        }
    }

    /// The states the kernel keeps while synchronised, a leap second
    /// pending or passing included, with `STA_UNSYNC` clear.
    #[test]
    fn a_kept_state_with_the_unsync_bit_clear_is_synchronised() {
        for clock in KEPT {
            assert_eq!(
                classified(clock, libc::STA_PLL, 0),
                ClockSync::Synced,
                "{clock}"
            );
        }
    }

    #[test]
    fn the_unsync_bit_makes_every_kept_state_unsynchronised() {
        for clock in KEPT {
            let why = unsynced(clock, libc::STA_UNSYNC | libc::STA_PLL, 0);
            assert!(
                why.contains("STA_UNSYNC") && why.contains(NO_DAEMON),
                "{clock}: {why}"
            );
        }
    }

    /// What the kernel answers with no time daemon: `TIME_ERROR` with
    /// `STA_UNSYNC` set. The bit is read first, so it says why.
    #[test]
    fn a_clock_error_with_the_unsync_bit_names_the_missing_daemon() {
        let why = unsynced(libc::TIME_ERROR, libc::STA_UNSYNC, 0);
        assert!(why.contains(NO_DAEMON), "{why}");
    }

    #[test]
    fn a_clock_error_is_unsynchronised() {
        let why = unsynced(libc::TIME_ERROR, 0, 0);
        assert!(why.contains("TIME_ERROR"), "{why}");
    }

    /// Anything not affirmed is unsynchronised: a failed call (its errno
    /// said), and a state this build does not know.
    #[test]
    fn a_failed_call_or_an_unknown_state_is_unsynchronised() {
        let failed = unsynced(-1, 0, libc::EPERM);
        let said = std::io::Error::from_raw_os_error(libc::EPERM).to_string();
        assert!(
            failed.contains("could not be read") && failed.contains(&said),
            "{failed}"
        );
        assert!(unsynced(42, 0, 0).contains("42"));
        assert!(unsynced(-2, 0, 0).contains("-2"));
    }
}
