use super::*;

#[test]
fn the_first_edit_is_sent_and_its_own_answer_is_fresh() {
    let mut flight = SearchFlight::default();
    assert_eq!(flight.edited(), Some(1));
    flight.sent("s-1".into(), 1);
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
    flight.sent("s-1".into(), first);
    assert_eq!(flight.edited(), None, "single flight");
    assert_eq!(flight.edited(), None);
    assert_eq!(
        flight.settle(Some("s-1"), Some(first)),
        Settled::Stale { resend: true }
    );
    assert_eq!(flight.latest(), 3);
    flight.sent("s-2".into(), flight.latest());
    assert_eq!(flight.settle(Some("s-2"), Some(3)), Settled::Fresh);
}

#[test]
fn an_answer_is_fresh_only_under_the_sent_id_with_the_sent_generation() {
    let mut flight = SearchFlight::default();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation);
    for foreign in [None, Some("s-0"), Some("tab2:s-1"), Some("")] {
        assert_eq!(flight.settle(foreign, Some(generation)), Settled::Foreign);
        assert!(flight.is_in_flight(), "a foreign answer settles nothing");
    }
    // The right id with a wrong or missing echo is never shown.
    assert_eq!(
        flight.settle(Some("s-1"), Some(generation + 1)),
        Settled::Stale { resend: false }
    );
    let generation = flight.edited().unwrap();
    flight.sent("s-2".into(), generation);
    assert_eq!(
        flight.settle(Some("s-2"), None),
        Settled::Stale { resend: false }
    );
}

#[test]
fn a_superseding_edit_makes_the_flight_stale_and_queues_nothing() {
    let mut flight = SearchFlight::default();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation);
    flight.edited();
    flight.superseded();
    assert_eq!(
        flight.settle(Some("s-1"), Some(generation)),
        Settled::Stale { resend: false }
    );
    // Superseded with nothing in flight: the next edit goes straight out.
    flight.superseded();
    assert_eq!(flight.edited(), Some(flight.latest()));
}

#[test]
fn a_failed_send_frees_the_flight() {
    let mut flight = SearchFlight::default();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation);
    flight.unsent();
    assert!(!flight.is_in_flight() && !flight.owns(Some("s-1")));
    assert_eq!(flight.edited(), Some(2));
}

#[test]
fn an_abandoned_flight_is_foreign_and_the_next_edit_goes_straight_out() {
    let mut flight = SearchFlight::default();
    let generation = flight.edited().unwrap();
    flight.sent("s-1".into(), generation);
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
