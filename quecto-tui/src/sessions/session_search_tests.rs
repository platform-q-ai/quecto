use super::*;

fn now() -> Instant {
    Instant::now()
}

#[test]
fn the_first_edit_is_sent_and_its_own_answer_is_fresh() {
    let mut flight = SearchFlight::default();
    assert_eq!(flight.edited(), Some(1));
    flight.sent("s-1".into(), 1, now());
    assert!(flight.is_in_flight() && flight.owns(Some("s-1")));
    assert_eq!(flight.settle(Some("s-1"), Some(1)), Settled::Fresh);
    assert!(!flight.is_in_flight());
    assert_eq!(
        flight.settle(Some("s-1"), Some(1)),
        Settled::Foreign,
        "settled once"
    );
}

#[test]
fn edits_during_a_flight_are_queued_and_the_overtaken_answer_is_stale() {
    let mut flight = SearchFlight::default();
    let first = flight.edited().unwrap();
    flight.sent("s-1".into(), first, now());
    assert_eq!(flight.edited(), None, "single flight");
    assert_eq!(flight.edited(), None);
    assert_eq!(
        flight.settle(Some("s-1"), Some(first)),
        Settled::Stale {
            resend: true,
            progress: true
        }
    );
    assert_eq!(flight.latest(), 3);
    flight.sent("s-2".into(), flight.latest(), now());
    assert_eq!(flight.settle(Some("s-2"), Some(3)), Settled::Fresh);
}

#[test]
fn an_answer_is_fresh_only_under_the_sent_id_with_the_sent_generation() {
    let mut flight = SearchFlight::default();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation, now());
    for foreign in [None, Some("s-0"), Some("tab2:s-1"), Some("")] {
        assert_eq!(flight.settle(foreign, Some(generation)), Settled::Foreign);
        assert!(flight.is_in_flight(), "a foreign answer settles nothing");
    }
    // The right id with a wrong or missing echo is never shown.
    assert_eq!(
        flight.settle(Some("s-1"), Some(generation + 1)),
        Settled::Stale {
            resend: false,
            progress: false
        }
    );
    let generation = flight.edited().unwrap();
    flight.sent("s-2".into(), generation, now());
    assert_eq!(
        flight.settle(Some("s-2"), None),
        Settled::Stale {
            resend: false,
            progress: false
        }
    );
}

#[test]
fn a_superseding_edit_makes_the_flight_stale_and_queues_nothing() {
    let mut flight = SearchFlight::default();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation, now());
    flight.edited();
    flight.superseded();
    assert_eq!(
        flight.settle(Some("s-1"), Some(generation)),
        Settled::Stale {
            resend: false,
            progress: false
        }
    );
    // Superseded with nothing in flight: the next edit goes straight out.
    flight.superseded();
    assert_eq!(flight.edited(), Some(flight.latest()));
}

#[test]
fn a_failed_send_frees_the_flight() {
    let mut flight = SearchFlight::default();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation, now());
    flight.unsent();
    assert!(!flight.is_in_flight() && !flight.owns(Some("s-1")));
    assert_eq!(flight.edited(), Some(2));
}

#[test]
fn an_abandoned_flight_is_foreign_and_the_next_edit_goes_straight_out() {
    let mut flight = SearchFlight::default();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation, now());
    assert_eq!(flight.sent_generation(Some("s-1")), Some(generation));
    assert_eq!(flight.sent_generation(Some("s-2")), None);
    assert_eq!(flight.sent_generation(None), None);
    flight.edited();
    flight.abandon();
    assert_eq!(
        flight.settle(Some("s-1"), Some(generation)),
        Settled::Foreign
    );
    assert_eq!(
        flight.edited(),
        Some(flight.latest()),
        "no queue behind an abandoned flight"
    );
}

/// R1-T3: monotonic progress — an overtaken answer is newer than the rows on
/// screen, so it is worth showing; the flight stays unsettled until the
/// latest generation's answer arrives.
#[test]
fn an_overtaken_answer_is_progress_until_the_box_is_cleared_or_the_scope_changes() {
    let mut flight = SearchFlight::default();
    let first = flight.edited().unwrap();
    flight.sent("s-1".into(), first, now());
    assert_eq!(flight.state(), FlightState::Searching);
    flight.edited();
    assert_eq!(
        flight.settle(Some("s-1"), Some(first)),
        Settled::Stale {
            resend: true,
            progress: true
        }
    );
    assert_ne!(flight.state(), FlightState::Settled, "overtaken: unsettled");
    flight.sent("s-2".into(), flight.latest(), now());
    assert_eq!(flight.state(), FlightState::Searching);
    // A wrong echo is never progress.
    flight.edited();
    assert_eq!(
        flight.settle(Some("s-2"), Some(99)),
        Settled::Stale {
            resend: true,
            progress: false
        }
    );
    // Overtaken by a scope change: the old scope's rows are never shown.
    flight.sent("s-3".into(), flight.latest(), now());
    assert_eq!(flight.scope_changed(), None);
    assert_eq!(
        flight.settle(Some("s-3"), Some(3)),
        Settled::Stale {
            resend: true,
            progress: false
        }
    );
    // Overtaken by a cleared box (a listing): nothing of it is shown.
    flight.sent("s-4".into(), flight.latest(), now());
    flight.superseded();
    assert_eq!(
        flight.settle(Some("s-4"), Some(4)),
        Settled::Stale {
            resend: false,
            progress: false
        }
    );
    assert_eq!(
        flight.state(),
        FlightState::Settled,
        "a listing owns the rows"
    );
}

#[test]
fn the_state_is_settled_only_by_the_fresh_answer_of_the_latest_edit() {
    let mut flight = SearchFlight::default();
    assert_eq!(flight.state(), FlightState::Settled, "nothing was asked");
    let generation = flight.edited().unwrap();
    assert_eq!(flight.state(), FlightState::Stalled, "wanted, not yet sent");
    flight.sent("s-1".into(), generation, now());
    assert_eq!(flight.state(), FlightState::Searching);
    assert_eq!(flight.settle(Some("s-1"), Some(generation)), Settled::Fresh);
    assert_eq!(flight.state(), FlightState::Settled);
    // A failed send and a lost connection leave the text unanswered.
    let generation = flight.edited().unwrap();
    flight.sent("s-2".into(), generation, now());
    flight.unsent();
    assert_eq!(flight.state(), FlightState::Stalled);
    let generation = flight.edited().unwrap();
    flight.sent("s-3".into(), generation, now());
    flight.interrupted();
    assert_eq!(flight.state(), FlightState::Stalled);
    assert_eq!(
        flight.settle(Some("s-3"), Some(generation)),
        Settled::Foreign
    );
    assert_eq!(flight.edited(), Some(flight.latest()), "typing recovers");
    // Closing the picker wants nothing any more.
    flight.abandon();
    assert_eq!(flight.state(), FlightState::Settled);
}

/// R1-T4: a lost answer never wedges the box.
#[test]
fn an_unanswered_search_is_retried_once_then_given_up_and_typing_still_searches() {
    let mut flight = SearchFlight::default();
    let start = now();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation, start);
    assert_eq!(flight.deadline(), Some(start + ANSWER_TIMEOUT));
    assert_eq!(flight.overdue(start + ANSWER_TIMEOUT / 2), None);
    flight.edited();
    let late = start + ANSWER_TIMEOUT;
    assert_eq!(flight.overdue(late), Some(Overdue::Retry(flight.latest())));
    assert!(!flight.is_in_flight() && flight.deadline().is_none());
    assert_eq!(
        flight.settle(Some("s-1"), Some(generation)),
        Settled::Foreign
    );
    flight.sent("s-2".into(), flight.latest(), late);
    assert_eq!(flight.overdue(late + ANSWER_TIMEOUT), Some(Overdue::GaveUp));
    assert_eq!(flight.state(), FlightState::Stalled);
    assert_eq!(
        flight.overdue(late + ANSWER_TIMEOUT * 2),
        None,
        "nothing in flight"
    );
    // The next edit goes straight out and earns its own retry.
    let next = flight.edited().unwrap();
    flight.sent("s-3".into(), next, late);
    assert_eq!(
        flight.overdue(late + ANSWER_TIMEOUT),
        Some(Overdue::Retry(next))
    );
}

#[test]
fn an_overdue_search_nobody_wants_any_more_is_dropped_without_a_retry() {
    let mut flight = SearchFlight::default();
    let start = now();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation, start);
    flight.superseded();
    assert_eq!(flight.overdue(start + ANSWER_TIMEOUT), Some(Overdue::Moot));
    assert!(!flight.is_in_flight());
    assert_eq!(
        flight.edited(),
        Some(flight.latest()),
        "emptying the box recovered"
    );
}
