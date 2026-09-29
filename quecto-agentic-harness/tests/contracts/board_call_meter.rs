//! `BoardCallMeter` on the SQLite adapter (#2303): the work of one call
//! runs exactly once; a call that began no board transaction has no
//! measure (nothing measured is never reported as zero); a call's measure
//! is its own transactions' and nothing else's; a measure opened inside
//! another adds its waits to the outer one.
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::{BoardCallMeter, BoardRepository};
use quecto::infrastructure::persistence::swarm_board::meter::SqliteBoardCallMeter;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;

fn board() -> (tempfile::TempDir, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    (dir, repository)
}

#[test]
fn the_work_runs_exactly_once_and_unmeasured_is_none() {
    let meter = SqliteBoardCallMeter;
    let mut runs = 0;
    assert_eq!(meter.metered(&mut || runs += 1), None);
    assert_eq!(runs, 1);
}

#[test]
fn a_call_measures_its_own_transactions() {
    let (_dir, repository) = board();
    let meter = SqliteBoardCallMeter;
    let measure = meter
        .metered(&mut || {
            repository.atomic(false, &mut |_| Ok(())).unwrap();
            repository.atomic(false, &mut |_| Ok(())).unwrap();
        })
        .expect("two transactions began");
    assert_eq!(measure.transactions, 2, "{measure:?}");
    assert!(!measure.busy, "{measure:?}");
    assert_eq!(measure.busy_wait, Duration::ZERO, "{measure:?}");
    // A transaction outside any measure is no call's.
    repository.atomic(false, &mut |_| Ok(())).unwrap();
    let next = meter
        .metered(&mut || repository.atomic(false, &mut |_| Ok(())).unwrap())
        .unwrap();
    assert_eq!(next.transactions, 1, "{next:?}");
}

#[test]
fn a_nested_measure_adds_its_waits_to_the_outer_one() {
    let (dir, repository) = board();
    let meter = SqliteBoardCallMeter;
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
    let mut inner = None;
    let outer = meter
        .metered(&mut || {
            inner = meter.metered(&mut || {
                repository.atomic(false, &mut |_| Ok(())).unwrap_err();
            });
        })
        .expect("the inner call's transaction is the outer call's too");
    release.send(()).unwrap();
    holder.join().unwrap();
    let inner = inner.expect("the inner call began a transaction");
    assert!(inner.busy && outer.busy, "{outer:?}");
    assert_eq!(outer.transactions, inner.transactions);
    assert_eq!(outer.lock_wait, inner.lock_wait);
    assert_eq!(outer.busy_wait, inner.busy_wait);
    assert!(outer.busy_wait >= Duration::from_millis(500), "{outer:?}");
}
