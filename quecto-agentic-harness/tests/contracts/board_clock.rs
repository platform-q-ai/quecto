//! Contract for [`BoardClock`] (#2476), on the system clock: the time to
//! the second as the board spells it, and no outside reference yet.
use quecto::application::project_board::ports::BoardClock;
use quecto::infrastructure::time::board_clock::SystemBoardClock;

#[test]
fn the_system_clock_reads_this_machines_time_to_the_second() {
    let before = humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string();
    let now = SystemBoardClock.now();
    let after = humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string();
    assert!(
        before.as_str() <= now.as_str() && now.as_str() <= after.as_str(),
        "{before} {now:?} {after}"
    );
    assert_eq!(SystemBoardClock.reference(), None);
}
