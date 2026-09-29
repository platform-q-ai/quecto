//! `MeteredCall` on the SQLite adapter (#2303): a call is the repository
//! it measures. It has measured nothing until a transaction begins through
//! it, and a lock held for ~150 ms is measured as a lock wait and a busy
//! wait of at least 100 ms each (the criterion's number, round-3 review
//! L2).
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use quecto::application::swarm::ports::{BoardCallMeter, BoardRepository};

use super::board_call_meter::board;

#[test]
fn a_call_measures_the_transactions_begun_through_it() {
    let (_dir, _plain, meter) = board();
    let call = meter.open();
    assert_eq!(call.measure(), None, "nothing began");
    let repository: std::sync::Arc<dyn BoardRepository> = call.clone();
    repository.atomic(false, &mut |_| Ok(())).unwrap();
    assert_eq!(call.measure().unwrap().transactions, 1);
}

#[test]
fn a_held_lock_is_measured_as_its_wait() {
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
    let releaser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        release.send(()).unwrap();
    });
    let call = meter.open();
    call.atomic(false, &mut |_| Ok(())).unwrap();
    releaser.join().unwrap();
    holder.join().unwrap();
    let measure = call.measure().expect("a transaction began");
    assert!(measure.busy, "{measure:?}");
    assert!(
        measure.lock_wait >= Duration::from_millis(100),
        "{measure:?}"
    );
    assert!(
        measure.busy_wait >= Duration::from_millis(100),
        "{measure:?}"
    );
}
