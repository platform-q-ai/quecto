use super::*;
use crate::infrastructure::tools::launch_rollbacks::InFlight;

#[tokio::test]
async fn an_exit_waits_for_a_rollback_in_flight() {
    let work = InFlight::enter();
    let started = tokio::time::Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(work);
    });
    assert!(await_launch_rollbacks(Duration::from_secs(30)).await);
    assert!(started.elapsed() >= Duration::from_millis(300));
}

#[tokio::test]
async fn an_exit_gives_up_at_its_limit() {
    let work = InFlight::enter();
    assert!(!await_launch_rollbacks(Duration::from_millis(100)).await);
    drop(work);
}
