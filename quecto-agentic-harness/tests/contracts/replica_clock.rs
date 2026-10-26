//! Contract for [`ReplicaClock`] (epic #2705 B11c): the monotonic clock a
//! follower times a stall by starts near zero and never goes back.
use quecto::application::project_board::ports::replication::ReplicaClock;
use quecto::infrastructure::time::replica_clock::MonotonicReplicaClock;

#[test]
fn the_clock_starts_near_zero_and_never_goes_back() {
    let clock = MonotonicReplicaClock::default();
    let first = clock.now();
    assert!(first < std::time::Duration::from_secs(60), "{first:?}");
    let mut last = first;
    for _ in 0..1_000 {
        let now = clock.now();
        assert!(now >= last, "{now:?} after {last:?}");
        last = now;
    }
}
