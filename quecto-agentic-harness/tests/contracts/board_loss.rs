//! The loss and death statements on the SQLite adapter (#2277, #1961):
//! `BoardMembers`' launcher read and per-member loss scan, `BoardEvents`'
//! loss observations, `BoardTasks`' blocking of a dead member's work,
//! `BoardFiles`' reservations by owner and `BoardRuns`' failed hold, each
//! by Python's SQL (`swarm.py`, `swarm_repository.py`), the caller's
//! member bound as Python's `sqlite3` binds it.
use quecto::application::swarm::dto::{BoardLocation, ScopeObservation};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::{BoardError, RefusalKind};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::{Value, json};

fn board(seed: &str) -> (tempfile::TempDir, std::path::PathBuf, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(seed)
        .unwrap();
    (dir, database, repository)
}

fn within(
    repository: &SqliteBoardRepository,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) -> Result<(), BoardError> {
    repository.atomic(false, &mut |transaction| work(transaction))
}

fn rows(database: &std::path::Path, sql: &str) -> Vec<String> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let mut statement = connection.prepare(sql).unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Python's `sqlite3` refusal of a list bound at `position`.
fn list(position: usize) -> BoardError {
    BoardError::new(
        RefusalKind::Invalid,
        format!(
            "coordination store unavailable or contended: \
             Error binding parameter {position}: type 'list' is not supported"
        ),
    )
}

const MEMBERS: &str = "
    INSERT INTO members(id,reservation,status,launcher) VALUES('parent','p','live',NULL);
    INSERT INTO members(id,reservation,status,launcher) VALUES('worker','w','live','parent');
    INSERT INTO members(id,reservation,status,launcher) VALUES('5','f','live',5);";

#[test]
fn a_launcher_is_read_by_the_member_id_as_bound() {
    let (_dir, _database, repository) = board(MEMBERS);
    within(&repository, |transaction| {
        assert_eq!(
            transaction.member_launcher(&json!("worker"))?,
            Some(Some("parent".to_owned()))
        );
        assert_eq!(transaction.member_launcher(&json!("parent"))?, Some(None));
        assert_eq!(transaction.member_launcher(&json!("stranger"))?, None);
        assert_eq!(transaction.member_launcher(&Value::Null)?, None);
        // `id=5` finds `'5'` through TEXT affinity; a launcher stored as
        // a number is not text.
        assert_eq!(transaction.member_launcher(&json!(5))?, Some(None));
        Ok(())
    })
    .unwrap();
    let refused = within(&repository, |transaction| {
        transaction.member_launcher(&json!(["worker"])).map(|_| ())
    });
    assert_eq!(refused, Err(list(1)));
}

/// `lost_after_activation`: the latest `scope_unknown` naming the member
/// newer than the latest `activated` naming it, a detail's `member`
/// compared by Python's `==` (`5 == 5.0 == True`-style numbers, text only
/// with text), and a detail that is not an object naming no member.
#[test]
fn a_member_is_lost_when_its_latest_loss_follows_its_latest_activation() {
    let (_dir, _database, repository) = board(
        r#"
        INSERT INTO events(actor,time,action,detail) VALUES('p',1.0,'activated','{"member":"worker"}');
        INSERT INTO events(actor,time,action,detail) VALUES('p',2.0,'scope_unknown','{"member":"worker"}');
        INSERT INTO events(actor,time,action,detail) VALUES('p',3.0,'scope_unknown','{"member":"back"}');
        INSERT INTO events(actor,time,action,detail) VALUES('p',4.0,'activated','{"member":"back"}');
        INSERT INTO events(actor,time,action,detail) VALUES('p',5.0,'scope_unknown','{"member":5}');
        INSERT INTO events(actor,time,action,detail) VALUES('p',6.0,'scope_unknown','[1]');
        INSERT INTO events(actor,time,action,detail) VALUES('p',7.0,'scope_observed','{"member":"seen"}');"#,
    );
    within(&repository, |transaction| {
        for (member, lost) in [
            (json!("worker"), true),
            (json!("back"), false),
            (json!("never"), false),
            (json!("seen"), false),
            (json!(5), true),
            (json!(5.0), true),
            (json!("5"), false),
        ] {
            assert_eq!(
                transaction.lost_after_activation(&member)?,
                lost,
                "{member}"
            );
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn loss_observations_are_read_in_id_order() {
    let (_dir, _database, repository) = board(
        r#"
        INSERT INTO events(actor,time,action,detail) VALUES('b',2.5,'scope_observed','{"member":"w"}');
        INSERT INTO events(actor,time,action,detail) VALUES('a',1.0,'scope_unknown','{"member":"w"}');
        INSERT INTO events(actor,time,action,detail) VALUES('a',3.0,'scope_observed','{"member":7}');
        INSERT INTO events(actor,time,action,detail) VALUES(NULL,NULL,'scope_observed','{}');
        INSERT INTO events(actor,time,action,detail) VALUES('c',4,'scope_observed','"w"');
        INSERT INTO events(actor,time,action,detail) VALUES('d','soon','scope_observed','{"member":"w"}');"#,
    );
    within(&repository, |transaction| {
        let observation = |actor: Option<&str>, time: Value, member: Value| ScopeObservation {
            actor: actor.map(str::to_owned),
            time,
            member,
        };
        assert_eq!(
            transaction.scope_observations()?,
            [
                observation(Some("b"), json!(2.5), json!("w")),
                observation(Some("a"), json!(3.0), json!(7)),
                observation(None, Value::Null, Value::Null),
                // REAL affinity stores the integer as a REAL.
                observation(Some("c"), json!(4.0), Value::Null),
                // Text that is not a number stays text.
                observation(Some("d"), json!("soon"), json!("w")),
            ]
        );
        Ok(())
    })
    .unwrap();
}

/// `_confirmed_dead`'s writes: the member's active tasks block with the
/// blocker, its reservations are counted and, on an orderly exit,
/// deleted; nothing else changes.
#[test]
fn a_dead_members_work_blocks_and_its_reservations_go_by_owner() {
    let (_dir, database, repository) = board(
        "INSERT INTO tasks(id,status,owner) VALUES(1,'claimed','worker');
         INSERT INTO tasks(id,status,owner) VALUES(2,'submitted','worker');
         INSERT INTO tasks(id,status,owner) VALUES(3,'completed','worker');
         INSERT INTO tasks(id,status,owner) VALUES(4,'claimed','other');
         INSERT INTO tasks(id,status,owner,blocker) VALUES(5,'blocked','worker','x');
         INSERT INTO files VALUES('a',1,'worker','c','r');
         INSERT INTO files VALUES('b',2,'worker','c','r');
         INSERT INTO files VALUES('c',4,'other','c','r');",
    );
    within(&repository, |transaction| {
        transaction.block_owned_tasks(&json!("worker"), "dead")?;
        assert_eq!(transaction.owner_file_count(&json!("worker"))?, 2);
        assert_eq!(transaction.owner_file_count(&json!("nobody"))?, 0);
        transaction.delete_owner_files(&json!("worker"))?;
        assert_eq!(transaction.owner_file_count(&json!("worker"))?, 0);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        rows(
            &database,
            "SELECT id||':'||status||':'||coalesce(blocker,'-') FROM tasks ORDER BY id"
        ),
        [
            "1:blocked:dead",
            "2:blocked:dead",
            "3:completed:-",
            "4:claimed:-",
            "5:blocked:dead"
        ]
    );
    assert_eq!(rows(&database, "SELECT path FROM files"), ["c"]);
    // The owner is parameter 2 of the block, and 1 of the count and the
    // delete, as Python numbers them.
    for (position, refused) in [
        (
            2,
            within(&repository, |transaction| {
                transaction.block_owned_tasks(&json!([]), "dead")
            }),
        ),
        (
            1,
            within(&repository, |transaction| {
                transaction.owner_file_count(&json!([])).map(|_| ())
            }),
        ),
        (
            1,
            within(&repository, |transaction| {
                transaction.delete_owner_files(&json!([]))
            }),
        ),
    ] {
        assert_eq!(refused, Err(list(position)));
    }
}

/// `_end_by_loss` on a pause holding no outcome: the outcome is `failed`
/// with the reason, and the status stays as it is.
#[test]
fn a_held_pause_takes_the_failed_outcome() {
    let (_dir, database, repository) =
        board("INSERT INTO run(id,status,deadline,coordinator) VALUES('r','paused',1.0,'parent');");
    within(&repository, |transaction| transaction.hold_failed("lost")).unwrap();
    assert_eq!(
        rows(
            &database,
            "SELECT status||':'||outcome||':'||outcome_reason FROM run"
        ),
        ["paused:failed:lost"]
    );
}
