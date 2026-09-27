//! #2192: a panic contained to a tool call while it held a swarm lock
//! leaves the lock poisoned; every later swarm call reads it as it was left.
use super::*;

#[test]
fn a_poisoned_swarm_lock_is_read_as_it_was_left() {
    let jobs = std::sync::Arc::new(std::sync::Mutex::new(vec![1, 2]));
    let held = jobs.clone();
    let _ = std::thread::spawn(move || {
        let mut guard = held.lock().unwrap();
        guard.push(3);
        panic!("a tool call panicked under the swarm lock");
    })
    .join();
    assert!(jobs.is_poisoned());
    assert_eq!(*recovered(&jobs), [1, 2, 3]);
    recovered(&jobs).push(4);
    assert_eq!(recovered(&jobs).len(), 4);
}
