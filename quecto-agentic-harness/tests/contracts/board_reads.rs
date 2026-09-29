//! The read models' statements on the SQLite adapter (#2277, #1969):
//! `BoardRuns`' run as `dict(row)`, `BoardMembers`' statuses of the
//! owners, `BoardEvents`' one grouped `max(time)` scan, event time and
//! event page, `BoardEvidence`' rows, and `BoardTasks`' id page, task
//! states and held-claim owners, each by Python's SQL (`swarm.py`,
//! `swarm_tasks.py`).
use quecto::application::swarm::dto::{BoardLocation, CountedTask, DictRow, LatestActivity};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::{BoardError, RefusalKind};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::{Value, json};

fn board(seed: &str) -> (tempfile::TempDir, SqliteBoardRepository) {
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
    (dir, repository)
}

fn within(
    repository: &SqliteBoardRepository,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) -> Result<(), BoardError> {
    repository.atomic(false, &mut |transaction| work(transaction))
}

fn dict(columns: &[(&str, Value)]) -> DictRow {
    DictRow {
        columns: columns
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    }
}

/// `dict(row)` of the run, every column in table order (the added ones
/// last), `constraints` and `criteria` loaded; `None` without a run.
#[test]
fn the_run_is_every_column_with_its_contract_loaded() {
    let (_dir, repository) = board("");
    within(&repository, |transaction| {
        assert_eq!(transaction.run_row()?, None);
        Ok(())
    })
    .unwrap();
    let (_dir, repository) = board(
        r#"INSERT INTO run VALUES('r','g','["c"]','[{"id":"t"}]','p','p',3,5.5,'running','failed',NULL);"#,
    );
    within(&repository, |transaction| {
        assert_eq!(
            transaction.run_row()?,
            Some(dict(&[
                ("id", json!("r")),
                ("goal", json!("g")),
                ("constraints", json!(["c"])),
                ("criteria", json!([{"id": "t"}])),
                ("coordinator", json!("p")),
                ("integrator", json!("p")),
                ("member_limit", json!(3)),
                ("deadline", json!(5.5)),
                ("status", json!("running")),
                ("outcome", json!("failed")),
                ("outcome_reason", Value::Null),
            ]))
        );
        Ok(())
    })
    .unwrap();
}

/// The owners' statuses, and the one grouped scan of their latest board
/// events: an owner without an event is absent, one whose events carry
/// no time reads NULL.
#[test]
fn owner_statuses_and_their_latest_activity_are_read_in_one_scan_each() {
    let (_dir, repository) = board(
        "INSERT INTO members(id,status) VALUES('w','live');
         INSERT INTO members(id,status) VALUES('d','dead');
         INSERT INTO members(id,status) VALUES('n',NULL);
         INSERT INTO members(id,status) VALUES('x','live');
         INSERT INTO events(actor,time,action,detail) VALUES('w',1.0,'a','{}');
         INSERT INTO events(actor,time,action,detail) VALUES('w',3.5,'a','{}');
         INSERT INTO events(actor,time,action,detail) VALUES('d',2.0,'a','{}');
         INSERT INTO events(actor,time,action,detail) VALUES('n',NULL,'a','{}');
         INSERT INTO events(actor,time,action,detail) VALUES('x',9.0,'a','{}');",
    );
    within(&repository, |transaction| {
        let mut statuses = transaction.member_statuses(&["w", "d", "n", "stranger"])?;
        statuses.sort();
        assert_eq!(
            statuses,
            [
                ("d".to_owned(), Some("dead".to_owned())),
                ("n".to_owned(), None),
                ("w".to_owned(), Some("live".to_owned())),
            ]
        );
        let mut latest = transaction.latest_activity(&["w", "d", "n", "stranger"])?;
        latest.sort_by(|left, right| left.actor.cmp(&right.actor));
        let at = |actor: &str, latest: Value| LatestActivity {
            actor: actor.to_owned(),
            latest,
        };
        assert_eq!(
            latest,
            [
                at("d", json!(2.0)),
                at("n", Value::Null),
                at("w", json!(3.5))
            ]
        );
        assert_eq!(transaction.event_time(2)?, Some(json!(3.5)));
        assert_eq!(transaction.event_time(99)?, None);
        Ok(())
    })
    .unwrap();
}

/// A page of the audit history: the rows after the cursor, each as
/// `dict(row)` with its detail the stored text; an `after` beyond
/// SQLite's integers is refused as Python's `sqlite3` refuses it.
#[test]
fn events_are_paged_after_the_cursor() {
    let (_dir, repository) = board(
        r#"INSERT INTO events(actor,time,action,detail) VALUES('p',1.0,'created','{"a":1}');
           INSERT INTO events(actor,time,action,detail) VALUES('p',2.0,'claimed','{}');
           INSERT INTO events(actor,time,action,detail) VALUES('w',3.0,'sent','{"b":[2]}');"#,
    );
    within(&repository, |transaction| {
        assert_eq!(
            transaction.event_page(1, 2)?,
            [
                dict(&[
                    ("id", json!(2)),
                    ("actor", json!("p")),
                    ("time", json!(2.0)),
                    ("action", json!("claimed")),
                    ("detail", json!("{}")),
                ]),
                dict(&[
                    ("id", json!(3)),
                    ("actor", json!("w")),
                    ("time", json!(3.0)),
                    ("action", json!("sent")),
                    ("detail", json!(r#"{"b":[2]}"#)),
                ]),
            ]
        );
        assert!(transaction.event_page(3, 5)?.is_empty());
        Ok(())
    })
    .unwrap();
    let beyond = within(&repository, |transaction| {
        transaction.event_page(u64::MAX, 5).map(|_| ())
    });
    assert!(
        matches!(&beyond, Err(error) if error.kind() == RefusalKind::Invalid
            && error.message().contains("Error binding parameter 1")),
        "{beyond:?}"
    );
}

/// The evidence rows, the task id page, every task's state and the owner
/// of every held claim.
#[test]
fn evidence_and_task_reads_follow_pythons_sql() {
    let (_dir, repository) = board(
        r#"INSERT INTO evidence VALUES('t','a.log','R1','command','p',1);
           INSERT INTO tasks(id,status,owner,dependencies) VALUES(3,'claimed','w','[1]');
           INSERT INTO tasks(id,status,owner,dependencies) VALUES(1,'completed','w','[]');
           INSERT INTO tasks(id,status,owner,dependencies) VALUES(2,'blocked',NULL,'[]');
           INSERT INTO tasks(id,status,owner,dependencies) VALUES(4,'submitted','v','[]');"#,
    );
    within(&repository, |transaction| {
        assert_eq!(
            transaction.evidence_rows()?,
            [dict(&[
                ("criterion", json!("t")),
                ("artifact", json!("a.log")),
                ("revision", json!("R1")),
                ("kind", json!("command")),
                ("actor", json!("p")),
                ("accepted", json!(1)),
            ])]
        );
        assert_eq!(transaction.task_ids(1, 2)?, [json!(2), json!(3)]);
        assert_eq!(transaction.task_ids(0, 50)?.len(), 4);
        let states = transaction.task_states()?;
        assert_eq!(
            states.first(),
            Some(&CountedTask {
                id: json!(1),
                status: json!("completed"),
                dependencies: json!([]),
            })
        );
        assert_eq!(states.len(), 4);
        let mut owners = transaction.claim_owners()?;
        owners.sort_by_key(|owner| owner.to_string());
        assert_eq!(owners, [json!("v"), json!("w")]);
        Ok(())
    })
    .unwrap();
}
