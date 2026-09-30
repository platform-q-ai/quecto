//! `BoardRepository::read` on the SQLite adapter (#2338): a read
//! transaction refuses every write, never creates a missing board, reads
//! one consistent state while a writer holds the write lock, and a
//! metered call measures it as one transaction that was not busy.
use std::sync::mpsc;
use std::time::{Duration, Instant};

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::{BoardCallMeter, BoardRepository, BoardTransaction};
use quecto::domain::swarm::{BoardError, RefusalKind};
use quecto::infrastructure::persistence::swarm_board::meter::SqliteBoardCallMeter;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

fn location(dir: &tempfile::TempDir) -> BoardLocation {
    BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    }
}

/// A created board holding one `created` event.
fn created() -> (tempfile::TempDir, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&location(&dir));
    repository
        .atomic(true, &mut |transaction| {
            transaction.event("parent", 1.0, "created", &json!({}))
        })
        .unwrap();
    (dir, repository)
}

fn read<T>(
    repository: &dyn BoardRepository,
    work: impl Fn(&dyn BoardTransaction) -> Result<T, BoardError>,
) -> Result<T, BoardError> {
    let mut value = None;
    repository.read(&mut |transaction| {
        value = Some(work(transaction)?);
        Ok(())
    })?;
    Ok(value.expect("the read ran its work"))
}

#[test]
fn a_read_refuses_every_write_and_leaves_the_board_as_it_was() {
    let (_dir, repository) = created();
    let refused = read(&repository, |transaction| {
        transaction.event("parent", 2.0, "noted", &json!({}))
    })
    .unwrap_err();
    assert_eq!(refused.kind(), RefusalKind::Store, "{refused}");
    assert_eq!(
        read(&repository, |transaction| transaction.event_generation()).unwrap(),
        1,
        "nothing was written"
    );
}

#[test]
fn a_read_never_creates_a_missing_board() {
    let dir = tempfile::tempdir().unwrap();
    let repository = SqliteBoardRepository::new(&location(&dir));
    let refused = read(&repository, |transaction| transaction.event_generation()).unwrap_err();
    assert_eq!(refused.kind(), RefusalKind::StoreMissing, "{refused}");
    assert!(!dir.path().join("swarm.sqlite").exists());
}

/// One consistent view: a writer holding the write lock with an
/// uncommitted event is neither waited for nor seen, and a read after its
/// commit sees it.
#[test]
fn a_read_sees_one_committed_state_without_waiting_for_a_writer() {
    let (dir, repository) = created();
    let path = location(&dir).database;
    let (held, taken) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let writer = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        connection
            .execute(
                "INSERT INTO events(actor,time,action,detail) VALUES ('w',2.0,'noted','{}')",
                [],
            )
            .unwrap();
        held.send(()).unwrap();
        let _told = released.recv_timeout(Duration::from_secs(30));
        connection.execute_batch("COMMIT").unwrap();
    });
    taken.recv_timeout(Duration::from_secs(30)).unwrap();
    let started = Instant::now();
    let during = read(&repository, |transaction| {
        let first = transaction.event_generation()?;
        let second = transaction.event_generation()?;
        Ok((first, second))
    })
    .unwrap();
    let waited = started.elapsed();
    drop(release);
    writer.join().unwrap();
    assert_eq!(during, (1, 1), "the committed state, twice alike");
    assert!(waited < Duration::from_millis(250), "{waited:?}");
    assert_eq!(
        read(&repository, |transaction| transaction.event_generation()).unwrap(),
        2,
        "the commit, once made"
    );
}

#[test]
fn a_metered_read_is_one_transaction() {
    let (_dir, repository) = created();
    let call = SqliteBoardCallMeter::new(repository).open();
    assert_eq!(
        read(&*call, |transaction| transaction.event_generation()).unwrap(),
        1
    );
    let measure = call.measure().expect("a transaction began");
    assert_eq!(measure.transactions, 1, "{measure:?}");
    assert!(!measure.busy, "{measure:?}");
}
