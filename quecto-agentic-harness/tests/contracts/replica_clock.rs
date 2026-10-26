//! Contract for [`ReplicaClock`] (epic #2705 B11c): the clock a follower
//! times a stall by never goes back, and counts time the machine spends
//! suspended (boot time, at least the monotonic clock), so a stall that
//! spans a suspend is never timed short.
use quecto::application::project_board::ports::replication::ReplicaClock;
use quecto::infrastructure::time::replica_clock::MonotonicReplicaClock;

/// The monotonic clock, which stops while the machine is suspended.
fn monotonic() -> std::time::Duration {
    let mut at = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a valid out-pointer.
    let read = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut at) };
    assert_eq!(read, 0);
    let seconds = u64::try_from(at.tv_sec).unwrap();
    std::time::Duration::new(seconds, u32::try_from(at.tv_nsec).unwrap())
}

#[test]
fn the_clock_never_goes_back_and_counts_suspended_time() {
    let clock = MonotonicReplicaClock;
    let mut last = clock.now();
    for _ in 0..1_000 {
        let before = monotonic();
        let now = clock.now();
        assert!(now >= last, "{now:?} after {last:?}");
        assert!(
            now >= before,
            "boot time {now:?} behind monotonic {before:?}"
        );
        last = now;
    }
}
