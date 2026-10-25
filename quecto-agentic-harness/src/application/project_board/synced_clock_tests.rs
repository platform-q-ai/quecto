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
        ReferenceStatus::NoReference("the kernel says so".into())
    );
}

/// A synchronised host clock is itself kept to an outside time: it is the
/// reference, so a change is bounded against it and carries no clock note.
#[test]
fn a_synchronised_host_clock_tells_the_time_and_is_its_own_reference() {
    let clock = gated(ClockSync::Synced);
    assert_eq!(clock.now(), Ok(at("08:00:00")));
    assert_eq!(clock.reference(), Some(at("08:00:00")));
    assert_eq!(clock.reference_status(), ReferenceStatus::Referenced);
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
