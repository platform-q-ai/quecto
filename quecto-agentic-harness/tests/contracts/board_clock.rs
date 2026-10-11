//! Contract for [`BoardClock`] (#2476), on the system clock: the time to
//! the second as the board spells it, no outside reference yet, and short
//! growing pauses between attempts.
use quecto::application::project_board::ports::BoardClock;
use quecto::infrastructure::time::board_clock::{SystemBoardClock, pause_for};
use std::time::Duration;

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

#[test]
fn pauses_are_short_jittered_and_grow_with_the_attempt() {
    assert!(pause_for(2) < Duration::from_millis(100));
    assert!(
        pause_for(3) >= Duration::from_millis(100) && pause_for(3) < Duration::from_millis(150)
    );
    assert!(pause_for(1_000) < Duration::from_millis(550), "bounded");
    let started = std::time::Instant::now();
    SystemBoardClock.pause(2);
    assert!(started.elapsed() >= Duration::from_millis(50));
}
