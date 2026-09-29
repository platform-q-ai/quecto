//! `BoardCallMeter` on the SQLite adapter (#2303): each call opens its own
//! measure, sharing nothing with another call; a fresh measure has
//! measured nothing (never reported as zero); only the transactions begun
//! through a call's own repository are its measure, whatever else runs on
//! the same thread or another.
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::{BoardCallMeter, BoardRepository};
use quecto::infrastructure::persistence::swarm_board::meter::SqliteBoardCallMeter;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;

pub(crate) fn board() -> (
    tempfile::TempDir,
    SqliteBoardRepository,
    SqliteBoardCallMeter,
) {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    let meter = SqliteBoardCallMeter::new(repository.clone());
    (dir, repository, meter)
}

fn transact(repository: &Arc<dyn BoardRepository>) {
    repository.atomic(false, &mut |_| Ok(())).unwrap();
}

#[test]
fn a_fresh_measure_has_measured_nothing() {
    let (_dir, _plain, meter) = board();
    assert_eq!(meter.open().measure(), None);
}

#[test]
fn a_call_measures_its_own_transactions() {
    let (_dir, plain, meter) = board();
    let call = meter.open();
    let other = meter.open();
    let repository = call.repository();
    transact(&repository);
    // Neither the plain repository's transaction nor another call's is
    // this call's, though they run on the same thread in between.
    plain.atomic(false, &mut |_| Ok(())).unwrap();
    transact(&other.repository());
    transact(&repository);
    let measure = call.measure().expect("two transactions began");
    assert_eq!(measure.transactions, 2, "{measure:?}");
    assert!(!measure.busy, "{measure:?}");
    assert_eq!(measure.busy_wait, Duration::ZERO, "{measure:?}");
    assert_eq!(other.measure().unwrap().transactions, 1);
    assert_eq!(meter.open().measure(), None, "the next call starts afresh");
}

#[test]
fn calls_on_other_threads_are_not_this_calls() {
    let (_dir, _plain, meter) = board();
    let call = meter.open();
    let meter = Arc::new(meter);
    let others: Vec<_> = (0..2)
        .map(|_| {
            let meter = meter.clone();
            thread::spawn(move || {
                let other = meter.open();
                for _ in 0..3 {
                    transact(&other.repository());
                }
                other.measure().unwrap().transactions
            })
        })
        .collect();
    transact(&call.repository());
    for other in others {
        assert_eq!(other.join().unwrap(), 3);
    }
    assert_eq!(call.measure().unwrap().transactions, 1);
}
