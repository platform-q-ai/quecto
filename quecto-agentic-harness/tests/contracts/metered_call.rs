//! `MeteredCall` on the SQLite adapter (#2303): a measure nested in
//! another is its own, and everything it measures (transactions, lock
//! wait, busy wait, busy) is added to the outer one, which sat through it.
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use quecto::application::swarm::ports::BoardCallMeter;

use super::board_call_meter::board;

#[test]
fn a_nested_measure_adds_its_waits_to_the_outer_one() {
    let (dir, _plain, meter) = board();
    let (held, taken) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let path = dir.path().join("swarm.sqlite");
    let holder = thread::spawn(move || {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        let _told = released.recv_timeout(Duration::from_secs(30));
        connection.execute_batch("COMMIT").unwrap();
    });
    taken.recv_timeout(Duration::from_secs(30)).unwrap();
    let outer = meter.open();
    let inner = outer.nested();
    inner
        .repository()
        .atomic(false, &mut |_| Ok(()))
        .unwrap_err();
    release.send(()).unwrap();
    holder.join().unwrap();
    let inner = inner.measure().expect("the inner call began a transaction");
    let outer = outer
        .measure()
        .expect("the inner call's transaction is the outer call's too");
    assert!(inner.busy && outer.busy, "{outer:?}");
    assert_eq!(outer.transactions, inner.transactions);
    assert_eq!(outer.lock_wait, inner.lock_wait);
    assert_eq!(outer.busy_wait, inner.busy_wait);
    assert!(outer.busy_wait >= Duration::from_millis(500), "{outer:?}");
}

#[test]
fn the_outer_measure_is_not_the_inners() {
    let (_dir, _plain, meter) = board();
    let outer = meter.open();
    let inner = outer.nested();
    outer.repository().atomic(false, &mut |_| Ok(())).unwrap();
    assert_eq!(inner.measure(), None, "the outer's transaction is its own");
    assert_eq!(outer.measure().unwrap().transactions, 1);
}
