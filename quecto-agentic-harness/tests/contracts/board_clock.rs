//! Contract for [`BoardClock`] (#2476, #2491): the system clock tells the
//! time to the second as the board spells it, with no outside reference
//! of its own, and short growing jittered pauses between attempts; the
//! clock referenced to a remote's `Date` bounds a change against it.
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
    assert!(matches!(
        SystemBoardClock::default().reference_status(),
        quecto::application::project_board::ports::ReferenceStatus::NoReference(_)
    ));
}

/// Its monotonic time starts near zero when it is made and never runs
/// back, whichever clock reads it, referenced to a remote or not.
#[test]
fn the_monotonic_time_starts_at_the_clock_and_never_runs_back() {
    let clocks: [Box<dyn BoardClock>; 2] = [
        Box::new(SystemBoardClock::default()),
        Box::new(
            quecto::infrastructure::time::remote_time::ReferencedBoardClock::for_remote(
                "/srv/local.git",
            ),
        ),
    ];
    for clock in clocks {
        let first = clock.elapsed();
        assert!(first < Duration::from_secs(5), "{first:?}");
        std::thread::sleep(Duration::from_millis(5));
        let second = clock.elapsed();
        assert!(
            second >= first + Duration::from_millis(5),
            "{first:?} {second:?}"
        );
    }
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

/// A one-request HTTP server whose `Date` is `offset_seconds` from now.
fn serves_date(offset_seconds: i64) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = stream.read(&mut [0u8; 1024]);
        let delta = Duration::from_secs(offset_seconds.unsigned_abs());
        let date = match offset_seconds >= 0 {
            true => SystemTime::now() + delta,
            false => SystemTime::now() - delta,
        };
        let answer = format!(
            "HTTP/1.1 200 OK\r\nDate: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            httpdate::fmt_http_date(date)
        );
        stream.write_all(answer.as_bytes()).unwrap();
    });
    url
}

/// Creates a task on a fresh board with the referenced clock reading the
/// `Date` a server `offset_seconds` from this machine answers.
fn create_against(
    offset_seconds: i64,
) -> Result<
    quecto::domain::project_board::entities::task::Task,
    quecto::application::project_board::dto::ProjectBoardError,
> {
    use quecto::application::project_board::dto::{NewBoardTask, NewBoardTaskStatus};
    use quecto::application::project_board::ports::ProjectBoardStore;
    use quecto::application::project_board::use_cases::CreateBoardTask;
    use quecto::domain::project_board::value_objects::slug::Slug;
    use quecto::domain::project_board::value_objects::vocabulary::TaskKind;
    use quecto::infrastructure::test_support::memory_board_store::MemoryBoardStore;
    use quecto::infrastructure::time::remote_time::{
        HttpDate, REFRESH_EVERY, ReferencedBoardClock,
    };
    use std::sync::Arc;
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let source = HttpDate::without_proxy(serves_date(offset_seconds));
    let clock = ReferencedBoardClock::new(Ok(Arc::new(source)), REFRESH_EVERY, |_| {});
    let create = CreateBoardTask::new(Arc::new(store), Arc::new(clock)).unwrap();
    create.execute(&NewBoardTask {
        id: Slug::parse("a").unwrap(),
        title: "A".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: NewBoardTaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        entry: quecto::domain::project_board::entities::task_parts::Entry::Sectioned,
    })
}

/// #2491: a change from a clock more than a minute ahead of the remote's
/// `Date` is refused; one behind it, or within the minute, is made.
#[test]
fn a_clock_ahead_of_the_remotes_date_is_refused_and_one_behind_is_not() {
    use quecto::application::project_board::dto::ProjectBoardError;
    let ahead = create_against(-300);
    assert!(
        matches!(ahead, Err(ProjectBoardError::ClockAhead { .. })),
        "{ahead:?}"
    );
    assert!(create_against(300).is_ok(), "a clock behind the remote");
    assert!(create_against(-30).is_ok(), "within the minute");
}
