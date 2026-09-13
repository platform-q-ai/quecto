use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// Tracks how many settlements are in flight at once.
struct Gauge {
    now: AtomicUsize,
    peak: AtomicUsize,
}

impl Gauge {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            now: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        })
    }

    fn enter(&self) {
        let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
    }

    fn leave(&self) {
        self.now.fetch_sub(1, Ordering::SeqCst);
    }
}

fn settlement(gauge: Arc<Gauge>, id: usize, gate: Arc<tokio::sync::Notify>) -> Settlement<usize> {
    Box::pin(async move {
        gauge.enter();
        gate.notified().await;
        gauge.leave();
        id
    })
}

#[tokio::test]
async fn never_more_than_the_bound_in_flight_and_every_output_collected() {
    let gauge = Gauge::new();
    let gate = Arc::new(tokio::sync::Notify::new());
    let queued: Vec<_> = (0..10)
        .map(|id| settlement(gauge.clone(), id, gate.clone()))
        .collect();
    let all = tokio::spawn(BoundedSettlement::new(3, queued));
    // Let the first batch park, then release them one at a time.
    for _ in 0..10 {
        tokio::task::yield_now().await;
        assert!(gauge.now.load(Ordering::SeqCst) <= 3);
        gate.notify_one();
    }
    let mut outputs = tokio::time::timeout(std::time::Duration::from_secs(5), all)
        .await
        .unwrap()
        .unwrap();
    outputs.sort_unstable();
    assert_eq!(outputs, (0..10).collect::<Vec<_>>());
    assert_eq!(gauge.peak.load(Ordering::SeqCst), 3);
    assert_eq!(gauge.now.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_empty_queue_resolves_immediately() {
    let outputs: Vec<usize> = BoundedSettlement::new(4, Vec::new()).await;
    assert!(outputs.is_empty());
}

#[tokio::test]
async fn ready_settlements_complete_in_one_poll_regardless_of_bound() {
    let queued: Vec<Settlement<usize>> = (0..5usize)
        .map(|id| Box::pin(async move { id }) as Settlement<usize>)
        .collect();
    let mut outputs = BoundedSettlement::new(1, queued).await;
    outputs.sort_unstable();
    assert_eq!(outputs, vec![0, 1, 2, 3, 4]);
}

#[test]
#[should_panic(expected = "a settlement bound of zero could never progress")]
fn a_zero_bound_is_refused() {
    drop(BoundedSettlement::<usize>::new(0, Vec::new()));
}
