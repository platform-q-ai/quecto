use super::DurablePrefixLatch;
use crate::application::sessions::ports::DurablePrefixObservation;

#[test]
fn latch_is_clean_until_latched_and_take_consumes_it_once() {
    let latch = DurablePrefixLatch::shared();
    assert!(!latch.take());
    latch.latch();
    latch.latch();
    assert!(latch.take());
    assert!(!latch.take());
}

#[test]
fn the_port_view_drains_the_same_latch() {
    let latch = DurablePrefixLatch::shared();
    let port: std::sync::Arc<dyn DurablePrefixObservation> = latch.clone();
    latch.latch();
    assert!(port.take_durable_prefix_dirty());
    assert!(!latch.take(), "the port consumed the latch");
    assert!(format!("{latch:?}").starts_with("DurablePrefixLatch"));
}
