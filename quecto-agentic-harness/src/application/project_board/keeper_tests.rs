use super::*;

/// Only a keeping that leaves the run open tells the session to end it;
/// one that folded the run back, found it ended or gone, or whose session
/// ended, tells nothing.
#[test]
fn only_a_keeping_that_leaves_the_run_open_tells_the_session() {
    for end in [
        KeeperEnd::FoldBackRefused,
        KeeperEnd::Truncated,
        KeeperEnd::SwarmLost,
    ] {
        assert!(end.leaves_run_open(), "{end:?}");
        assert_eq!(
            Look::ending(end.clone()).notice,
            Some(KeeperNotice::LeftOpen(end))
        );
    }
    for end in [
        KeeperEnd::Folded,
        KeeperEnd::Ended,
        KeeperEnd::Gone,
        KeeperEnd::SessionEnded,
    ] {
        assert!(!end.leaves_run_open(), "{end:?}");
        assert_eq!(Look::ending(end).notice, None);
    }
    assert_eq!(Look::default().next, KEEPER_PERIOD);
}

/// The bounds are ordered as the keeper relies on: a paused swarm is
/// looked at more often than a going one, a run is given longer to show
/// than a sighted swarm to stall, and a paused swarm longer still.
#[test]
fn the_keepers_bounds_are_ordered() {
    assert!(PAUSED_PERIOD < KEEPER_PERIOD);
    assert!(STALL_LIMIT < START_GRACE && START_GRACE < PAUSED_LIMIT);
    assert!(RENEW_BELOW_SECONDS > i64::try_from(KEEPER_PERIOD.as_secs()).unwrap());
}
