use super::*;

// The count is process-wide: other tests' launches may be counted beside
// this one's, which can only lengthen a wait, never shorten it.
#[tokio::test]
async fn settled_waits_for_counted_work_and_gives_up_at_its_limit() {
    // Other tests' launches may be in flight: wait them out first.
    assert!(settled(Duration::from_secs(60)).await);
    let work = InFlight::enter();
    assert!(!settled(Duration::from_millis(120)).await);
    let finisher = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(work);
    });
    assert!(settled(Duration::from_secs(30)).await);
    finisher.await.unwrap();
}
