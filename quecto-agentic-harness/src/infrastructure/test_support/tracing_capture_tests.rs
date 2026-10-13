//! The capture keeps what its own thread writes, at the levels it asks
//! for, whatever other threads write meanwhile.
use super::*;

#[test]
fn a_thread_keeps_its_own_events_at_its_levels() {
    let kept = traced(Level::INFO, || {
        tracing::info!(target: "capture_test", "kept info");
        tracing::debug!(target: "capture_test", "dropped debug");
        tracing::warn!(target: "capture_test", "kept warn");
    });
    assert!(
        kept.contains("kept info") && kept.contains("kept warn"),
        "{kept}"
    );
    assert!(!kept.contains("dropped debug"), "{kept}");
}

#[test]
fn another_threads_events_are_never_kept() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other = std::sync::Arc::clone(&barrier);
    let noisy = std::thread::spawn(move || {
        other.wait();
        for _ in 0..100 {
            tracing::info!(target: "capture_test", "from another thread");
        }
    });
    let kept = traced(Level::TRACE, || {
        barrier.wait();
        for _ in 0..100 {
            tracing::info!(target: "capture_test", "from this thread");
        }
    });
    noisy.join().unwrap();
    assert!(!kept.contains("another thread"), "{kept}");
    assert_eq!(kept.matches("from this thread").count(), 100);
}

/// The reviewer's race (round 3): a callsite first reached by a thread
/// with no capture armed is still captured on the next thread that arms.
#[test]
fn a_callsite_reached_first_without_a_capture_is_still_captured() {
    fn site() {
        tracing::info!(target: "capture_test", "the one callsite");
    }
    std::thread::spawn(site).join().unwrap();
    let kept = traced(Level::TRACE, site);
    assert!(kept.contains("the one callsite"), "{kept}");
}
