use super::*;

fn at(base: Instant, seconds: u64) -> Instant {
    base + Duration::from_secs(seconds)
}

#[test]
fn a_settled_feed_requests_at_once() {
    let base = Instant::now();
    let mut refresh = StatsRefresh::Settled;
    assert!(refresh.should_request(base));
    assert_eq!(refresh, StatsRefresh::sent_at(base));
}

#[test]
fn a_trigger_while_pending_waits_for_the_reply_then_follows_up_once() {
    let base = Instant::now();
    let mut refresh = StatsRefresh::sent_at(base);
    assert!(!refresh.should_request(at(base, 1)));
    assert!(!refresh.should_request(at(base, 2)));
    assert!(
        refresh.answered(at(base, 3)),
        "one follow-up for the triggers seen"
    );
    assert_eq!(refresh, StatsRefresh::sent_at(at(base, 3)));
    assert!(
        !refresh.answered(at(base, 4)),
        "the follow-up's reply settles it"
    );
    assert_eq!(refresh, StatsRefresh::Settled);
}

#[test]
fn a_reply_with_no_trigger_since_settles() {
    let base = Instant::now();
    let mut refresh = StatsRefresh::sent_at(base);
    assert!(!refresh.answered(at(base, 1)));
    assert_eq!(refresh, StatsRefresh::Settled);
    // An unsolicited reply (a peer's request, a busy snapshot) changes nothing.
    assert!(!refresh.answered(at(base, 2)));
    assert_eq!(refresh, StatsRefresh::Settled);
}

#[test]
fn a_request_unanswered_past_the_grace_may_be_sent_again() {
    let base = Instant::now();
    let mut refresh = StatsRefresh::sent_at(base);
    let just_before = base + STATS_REPLY_GRACE - Duration::from_millis(1);
    assert!(!refresh.should_request(just_before));
    assert!(refresh.should_request(base + STATS_REPLY_GRACE));
    assert_eq!(refresh, StatsRefresh::sent_at(base + STATS_REPLY_GRACE));
}

#[test]
fn a_request_that_could_not_be_queued_leaves_nothing_outstanding() {
    let base = Instant::now();
    let mut refresh = StatsRefresh::Settled;
    assert!(refresh.should_request(base));
    refresh.not_sent();
    assert_eq!(refresh, StatsRefresh::Settled);
    assert!(refresh.should_request(at(base, 1)));
}
