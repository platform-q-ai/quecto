//! Contract for the `DurablePrefixObservation` port (#1860, D5 #1972):
//! `take_durable_prefix_dirty` is read-and-clear over the agent loop's
//! shared latch, so exactly one observer consumes each latched change.
use std::sync::Arc;

use quecto::application::durable_prefix::DurablePrefixLatch;
use quecto::application::sessions::ports::DurablePrefixObservation;

fn under_test() -> (Arc<DurablePrefixLatch>, Arc<dyn DurablePrefixObservation>) {
    let latch = DurablePrefixLatch::shared();
    (latch.clone(), latch)
}

#[test]
fn a_fresh_latch_observes_clean() {
    let (_, port) = under_test();
    assert!(!port.take_durable_prefix_dirty());
}

#[test]
fn a_latched_change_is_observed_exactly_once() {
    let (latch, port) = under_test();
    latch.latch();
    assert!(port.take_durable_prefix_dirty());
    assert!(!port.take_durable_prefix_dirty());
    assert!(!latch.take(), "the observation consumed the latch");
}

#[test]
fn repeated_latching_before_an_observation_collapses_to_one() {
    let (latch, port) = under_test();
    latch.latch();
    latch.latch();
    assert!(port.take_durable_prefix_dirty());
    assert!(!port.take_durable_prefix_dirty());
}
