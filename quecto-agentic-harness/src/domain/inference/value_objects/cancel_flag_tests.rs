use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Wake, Waker};

/// A waker that counts how often it is woken.
#[derive(Default)]
struct Counting(AtomicUsize);
impl Wake for Counting {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn counting() -> (Arc<Counting>, Waker) {
    let count = Arc::new(Counting::default());
    (count.clone(), Waker::from(count))
}
fn wakes(count: &Counting) -> usize {
    count.0.load(Ordering::SeqCst)
}

#[test]
fn a_flag_cancelled_before_the_watch_is_seen_without_registering() {
    let flag = CancelFlag::new();
    flag.cancel();
    let (count, waker) = counting();
    let mut watch = flag.watch();
    assert!(watch.cancelled_or_wake(&waker));
    assert_eq!(flag.waiters(), 0);
    assert_eq!(wakes(&count), 0);
}

#[test]
fn a_pending_watch_is_woken_once_by_cancel_and_never_before() {
    let flag = CancelFlag::new();
    let (count, waker) = counting();
    let mut watch = flag.watch();
    assert!(!watch.cancelled_or_wake(&waker));
    // Watching again with the same waker does not register it twice.
    assert!(!watch.cancelled_or_wake(&waker));
    assert_eq!(flag.waiters(), 1);
    assert_eq!(wakes(&count), 0, "no wake without cancel");
    flag.clone().cancel();
    assert_eq!(wakes(&count), 1);
    assert_eq!(flag.waiters(), 0, "cancel releases every watch");
    assert!(watch.cancelled_or_wake(&waker));
    // A second cancel has nobody left to wake.
    flag.cancel();
    assert_eq!(wakes(&count), 1);
    assert!(flag.is_cancelled());
}

#[test]
fn cancel_wakes_every_watch_of_every_clone() {
    let flag = CancelFlag::new();
    let clone = flag.clone();
    let wakers: Vec<_> = (0..3).map(|_| counting()).collect();
    let mut watches = vec![flag.watch(), clone.watch(), flag.watch()];
    for (watch, (_, waker)) in watches.iter_mut().zip(&wakers) {
        assert!(!watch.cancelled_or_wake(waker));
    }
    assert_eq!(flag.waiters(), 3);
    clone.cancel();
    for ((count, waker), watch) in wakers.iter().zip(&mut watches) {
        assert_eq!(wakes(count), 1);
        assert!(watch.cancelled_or_wake(waker));
    }
    assert_eq!(flag.waiters(), 0);
}

#[test]
fn a_new_waker_replaces_the_registered_one() {
    let flag = CancelFlag::new();
    let (first, first_waker) = counting();
    let (second, second_waker) = counting();
    let mut watch = flag.watch();
    assert!(!watch.cancelled_or_wake(&first_waker));
    assert!(!watch.cancelled_or_wake(&second_waker));
    assert_eq!(flag.waiters(), 1);
    flag.cancel();
    assert_eq!(wakes(&first), 0, "the replaced waker is gone");
    assert_eq!(wakes(&second), 1);
}

#[test]
fn a_dropped_watch_leaves_no_waiter_behind() {
    let flag = CancelFlag::new();
    let (count, waker) = counting();
    for _ in 0..100 {
        let mut watch = flag.watch();
        assert!(!watch.cancelled_or_wake(&waker));
    }
    let mut other = flag.watch();
    assert!(!other.cancelled_or_wake(&waker));
    let mut kept = flag.watch();
    assert!(!kept.cancelled_or_wake(&waker));
    drop(flag.watch());
    drop(other);
    assert_eq!(flag.waiters(), 1, "only the live watch stays registered");
    flag.cancel();
    assert_eq!(wakes(&count), 1);
    drop(kept);
    assert_eq!(flag.waiters(), 0);
}

/// No lost wake-up: a cancel racing a watch's registration from another
/// thread always wakes the watch, or the watch sees it.
#[test]
fn a_cancel_racing_the_registration_is_never_lost() {
    for _ in 0..2_000 {
        let flag = CancelFlag::new();
        let canceller = flag.clone();
        let (count, waker) = counting();
        let mut watch = flag.watch();
        let cancel = std::thread::spawn(move || canceller.cancel());
        let seen = watch.cancelled_or_wake(&waker);
        cancel.join().unwrap();
        assert!(
            seen || wakes(&count) == 1,
            "the cancel is seen or wakes the watch"
        );
        assert!(watch.cancelled_or_wake(&waker));
        assert_eq!(flag.waiters(), 0);
    }
}

/// No lost wake-up, however the registration interleaves with the cancel:
/// a watcher registers watch after watch while another thread cancels, and
/// every watch that did not see the cancel was woken by it.
#[test]
fn every_watch_registered_around_a_cancel_is_woken_or_sees_it() {
    for _ in 0..200 {
        let flag = CancelFlag::new();
        let canceller = flag.clone();
        let watcher = flag.clone();
        let start = Arc::new(std::sync::Barrier::new(2));
        let go = start.clone();
        let watching = std::thread::spawn(move || {
            let mut pending = Vec::new();
            go.wait();
            loop {
                let (count, waker) = counting();
                let mut watch = watcher.watch();
                if watch.cancelled_or_wake(&waker) {
                    break;
                }
                pending.push((count, watch));
            }
            pending
        });
        start.wait();
        std::thread::yield_now();
        canceller.cancel();
        let pending = watching.join().unwrap();
        for (count, _) in &pending {
            assert_eq!(wakes(count), 1, "a pending watch missed the cancel");
        }
        assert_eq!(flag.waiters(), 0);
    }
}
