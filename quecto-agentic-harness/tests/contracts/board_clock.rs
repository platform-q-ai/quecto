//! Contract for [`BoardClock`] (#2476), on the system clock: the time to
//! the second as the board spells it, no outside reference yet, and short
//! growing jittered pauses between attempts.
use quecto::application::project_board::ports::{BoardClock, BoardClockError};
use quecto::infrastructure::time::board_clock::{SystemBoardClock, timestamp_at};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn the_system_clock_reads_this_machines_time_to_the_second() {
    let before = humantime::format_rfc3339_seconds(SystemTime::now()).to_string();
    let now = SystemBoardClock::default().now().unwrap();
    let after = humantime::format_rfc3339_seconds(SystemTime::now()).to_string();
    assert!(
        before.as_str() <= now.as_str() && now.as_str() <= after.as_str(),
        "{before} {now:?} {after}"
    );
    assert_eq!(SystemBoardClock::default().reference(), None);
}

/// A clock set before 1970 or past 9999 is an error, not a panic.
#[test]
fn a_time_the_board_cannot_spell_is_an_error() {
    let before_1970 = UNIX_EPOCH - Duration::from_secs(1);
    assert!(
        matches!(
            timestamp_at(before_1970),
            Err(BoardClockError::OutOfRange(_))
        ),
        "before 1970"
    );
    let past_9999 = UNIX_EPOCH + Duration::from_secs(253_402_300_800);
    assert!(
        matches!(timestamp_at(past_9999), Err(BoardClockError::OutOfRange(_))),
        "past 9999"
    );
    assert_eq!(
        timestamp_at(UNIX_EPOCH).unwrap().as_str(),
        "1970-01-01T00:00:00Z"
    );
}

#[test]
fn pauses_are_short_and_grow_with_the_attempt() {
    let clock = SystemBoardClock::default();
    assert!(clock.pause_for(2) < Duration::from_millis(100));
    let third = clock.pause_for(3);
    assert!(
        third >= Duration::from_millis(100) && third < Duration::from_millis(150),
        "{third:?}"
    );
    assert!(
        clock.pause_for(1_000) < Duration::from_millis(550),
        "bounded"
    );
    let started = std::time::Instant::now();
    clock.pause(2);
    assert!(started.elapsed() >= Duration::from_millis(50));
}

/// Two pauses before the same attempt differ, each within its step, and a
/// seed replays the same pauses.
#[test]
fn pauses_for_the_same_attempt_are_jittered_within_one_step() {
    let pauses = |clock: &SystemBoardClock| -> Vec<Duration> {
        (0..16).map(|_| clock.pause_for(3)).collect()
    };
    let drawn = pauses(&SystemBoardClock::seeded(7));
    assert_eq!(
        drawn,
        pauses(&SystemBoardClock::seeded(7)),
        "a seed replays"
    );
    assert!(
        drawn.windows(2).all(|pair| pair[0] != pair[1]),
        "successive pauses differ: {drawn:?}"
    );
    assert!(
        drawn.iter().all(
            |pause| *pause >= Duration::from_millis(100) && *pause < Duration::from_millis(150)
        ),
        "{drawn:?}"
    );
}
