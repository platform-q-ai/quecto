use super::*;

const PERIOD: u64 = 1_000;

fn watch() -> IdleWatch {
    IdleWatch::new(PERIOD).expect("a positive period")
}

#[test]
fn a_zero_period_is_refused() {
    assert_eq!(IdleWatch::new(0), None);
    assert!(IdleWatch::new(1).is_some());
}

#[test]
fn a_broker_idle_for_the_whole_period_stops() {
    let mut watch = watch();
    assert_eq!(
        watch.observe(false, 10),
        IdleVerdict::IdleUntil(10 + PERIOD)
    );
    assert_eq!(
        watch.observe(false, 500),
        IdleVerdict::IdleUntil(10 + PERIOD)
    );
    assert_eq!(
        watch.observe(false, 10 + PERIOD - 1),
        IdleVerdict::IdleUntil(10 + PERIOD)
    );
    assert_eq!(watch.observe(false, 10 + PERIOD), IdleVerdict::Stop);
}

#[test]
fn a_broker_that_holds_anything_never_stops_and_restarts_the_period() {
    let mut watch = watch();
    assert_eq!(watch.observe(false, 0), IdleVerdict::IdleUntil(PERIOD));
    // Holding past the first deadline never stops it.
    assert_eq!(watch.observe(true, PERIOD), IdleVerdict::Holding);
    assert_eq!(watch.observe(true, 10 * PERIOD), IdleVerdict::Holding);
    // Idle again: a whole new period from the moment it let go.
    assert_eq!(
        watch.observe(false, 10 * PERIOD + 1),
        IdleVerdict::IdleUntil(11 * PERIOD + 1)
    );
    assert_eq!(
        watch.observe(false, 11 * PERIOD),
        IdleVerdict::IdleUntil(11 * PERIOD + 1)
    );
}

#[test]
fn the_deadline_saturates_instead_of_wrapping() {
    let mut watch = watch();
    assert_eq!(
        watch.observe(false, u64::MAX - 1),
        IdleVerdict::IdleUntil(u64::MAX)
    );
    assert_eq!(watch.observe(false, u64::MAX), IdleVerdict::Stop);
}

#[test]
fn an_always_on_broker_never_stops() {
    let mut lifetime = AuthorityLifetime::AlwaysOn;
    assert_eq!(lifetime.observe(false, 0), IdleVerdict::Holding);
    assert_eq!(lifetime.observe(false, u64::MAX), IdleVerdict::Holding);
    let mut on_demand = AuthorityLifetime::StopWhenIdle(watch());
    assert_eq!(on_demand.observe(false, 0), IdleVerdict::IdleUntil(PERIOD));
    assert_eq!(on_demand.observe(false, PERIOD), IdleVerdict::Stop);
}
