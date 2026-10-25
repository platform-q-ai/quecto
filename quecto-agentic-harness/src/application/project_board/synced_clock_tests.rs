use super::*;
use crate::application::project_board::ports::ClockSync;
use crate::application::project_board::use_cases::fixtures_tests::{SetClock, at};

/// The host's word, as set.
struct Said(ClockSync);

impl TimeSync for Said {
    fn sync(&self) -> ClockSync {
        self.0.clone()
    }
}

fn gated(sync: ClockSync) -> SyncedBoardClock {
    SyncedBoardClock::new(SetClock::at("08:00:00"), Arc::new(Said(sync)))
}

#[test]
fn an_unsynchronised_host_clock_tells_no_time() {
    let clock = gated(ClockSync::Unsynced("the kernel says so".into()));
    assert_eq!(
        clock.now(),
        Err(BoardClockError::Unsynced("the kernel says so".into()))
    );
    assert_eq!(clock.reference(), None);
    assert_eq!(
        clock.reference_status(),
        ReferenceStatus::NoReference("set without one".into())
    );
}

/// A synchronised host clock tells the time, and is no outside reference
/// (B7's behaviour until B9): its reference is the wrapped clock's.
#[test]
fn a_synchronised_host_clock_tells_the_time_with_the_clocks_own_reference() {
    let clock = gated(ClockSync::Synced);
    assert_eq!(clock.now(), Ok(at("08:00:00")));
    assert_eq!(clock.reference(), None);
    assert_eq!(
        clock.reference_status(),
        ReferenceStatus::NoReference("set without one".into())
    );
}

/// Only `now` asks the host: an op that reads the clock reads the kernel
/// once, whatever else it asks of the clock.
#[test]
fn only_reading_the_time_asks_the_host() {
    struct Counted(std::sync::atomic::AtomicUsize);
    impl TimeSync for Counted {
        fn sync(&self) -> ClockSync {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ClockSync::Synced
        }
    }
    let counted = Arc::new(Counted(Default::default()));
    let clock = SyncedBoardClock::new(SetClock::at("08:00:00"), Arc::clone(&counted) as _);
    clock.now().unwrap();
    let _ = (clock.reference(), clock.reference_status(), clock.elapsed());
    clock.pause(2);
    assert_eq!(counted.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn pauses_and_elapsed_time_are_the_clocks_own() {
    let inner = SetClock::at("08:00:00");
    let unsynced = Arc::new(Said(ClockSync::Unsynced("no".into())));
    let clock = SyncedBoardClock::new(Arc::clone(&inner) as Arc<dyn BoardClock>, unsynced);
    clock.pause(2);
    assert_eq!(*inner.pauses.lock().unwrap(), [2]);
    assert_eq!(clock.elapsed(), Duration::ZERO);
}

/// One op's clock asks the host once, however often the op reads the time.
#[test]
fn a_held_word_is_read_once() {
    struct Counted(std::sync::atomic::AtomicUsize);
    impl TimeSync for Counted {
        fn sync(&self) -> ClockSync {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ClockSync::Unsynced("drifting".into())
        }
    }
    let counted = Counted(Default::default());
    let held = Arc::new(HeldSync::read(&counted));
    let clock = SyncedBoardClock::new(SetClock::at("08:00:00"), held);
    for _ in 0..3 {
        assert_eq!(
            clock.now(),
            Err(BoardClockError::Unsynced("drifting".into()))
        );
    }
    assert_eq!(counted.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}
