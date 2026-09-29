//! Contract for [`ExternalAgentClock`] (#2287), proven on the production
//! `TokioExternalAgentClock`: its instants never go back, and a sleep
//! lasts at least its duration on the same scale.
use std::time::Duration;

use quecto::application::external_agent::ports::ExternalAgentClock;
use quecto::infrastructure::external_agents::clock::TokioExternalAgentClock;

#[tokio::test(start_paused = true)]
async fn a_sleep_advances_now_by_at_least_its_duration() {
    let clock = TokioExternalAgentClock::new();
    let start = clock.now();
    clock.sleep(Duration::from_millis(1500)).await;
    let after = clock.now();
    assert!(after >= start);
    assert!(after.0 - start.0 >= 1500, "{start:?} -> {after:?}");
    clock.sleep(Duration::from_secs(60)).await;
    assert!(clock.now().0 - after.0 >= 60_000);
}

#[tokio::test]
async fn now_never_goes_back_and_a_zero_sleep_resolves() {
    let clock = TokioExternalAgentClock::new();
    let mut last = clock.now();
    for _ in 0..3 {
        tokio::time::timeout(
            Duration::from_secs(5),
            clock.sleep(Duration::from_millis(5)),
        )
        .await
        .expect("a short sleep ends");
        let now = clock.now();
        assert!(now > last, "{last:?} -> {now:?}");
        last = now;
    }
    tokio::time::timeout(Duration::from_secs(5), clock.sleep(Duration::ZERO))
        .await
        .expect("a zero sleep resolves");
}
