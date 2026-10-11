//! `BoardProjectionReads` on the SQLite adapter (#2478): only the
//! allowlisted columns, each kind in its order and cut at its limit with
//! its whole count, the usage ledger's totals and the revision, in a read
//! transaction that creates nothing.
use quecto::application::swarm::dto::{BoardLocation, NewTask, ProjectionLimits, ProjectionRows};
use quecto::application::swarm::ports::{BoardRepository, BoardTransaction};
use quecto::domain::swarm::BoardError;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

const LIMITS: ProjectionLimits = ProjectionLimits {
    items: 2,
    files: 10,
    members: 10,
    evidence: 10,
};

fn board() -> (tempfile::TempDir, std::path::PathBuf, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    (dir, database, repository)
}

fn within(
    repository: &SqliteBoardRepository,
    mut work: impl FnMut(&dyn BoardTransaction) -> Result<(), BoardError>,
) {
    repository
        .atomic(true, &mut |transaction| work(transaction))
        .unwrap();
}

fn rows(repository: &SqliteBoardRepository) -> ProjectionRows {
    let mut rows = None;
    repository
        .read(&mut |transaction| {
            rows = Some(transaction.projection_rows(LIMITS)?);
            Ok(())
        })
        .unwrap();
    rows.unwrap()
}

fn tables(database: &std::path::Path) -> Vec<String> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn an_empty_board_reads_no_run_no_rows_and_zero_totals_creating_nothing() {
    let (_dir, database, repository) = board();
    within(&repository, |_| Ok(()));
    let before = tables(&database);
    let rows = rows(&repository);
    assert_eq!(
        (rows.revision, rows.generation, rows.run.is_none()),
        (0, 0, true)
    );
    assert_eq!(
        (
            rows.item_count,
            rows.file_count,
            rows.member_count,
            rows.evidence_count
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(
        (
            rows.budget,
            rows.observed_tokens,
            rows.unknown_usage_requests
        ),
        (None, json!(0), json!(0))
    );
    assert_eq!(tables(&database), before, "no usage table was made");
}

#[test]
fn rows_come_in_order_cut_at_their_limit_with_their_whole_count() {
    let (_dir, database, repository) = board();
    within(&repository, |transaction| {
        for title in ["first", "second", "third"] {
            transaction.insert_task(&NewTask {
                title: title.to_owned(),
                acceptance: json!(["passes"]),
                dependencies: json!([]),
            })?;
        }
        Ok(())
    });
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "INSERT INTO run (id, goal, criteria, coordinator, integrator, member_limit, deadline, status) VALUES ('run-1','ship','[]','b','b',2,5.0,'running');
             INSERT INTO members (id, reservation, status, pid, socket) VALUES ('b','res-b','live',7,'sock'), ('a','res-a','reserved',8,'sock');
             INSERT INTO events (actor, time, action, detail) VALUES ('b', 3.5, 'claimed', '{}');",
        )
        .unwrap();
    let rows = rows(&repository);
    let titles: Vec<_> = rows.items.iter().map(|item| item.title.clone()).collect();
    assert_eq!(
        (titles, rows.item_count),
        (vec![json!("first"), json!("second")], 3)
    );
    assert_eq!(rows.items[0].criteria, json!(["passes"]));
    let members: Vec<_> = rows
        .members
        .iter()
        .map(|member| (member.id.clone(), member.last_activity.clone()))
        .collect();
    assert_eq!(
        members,
        [(json!("a"), json!(null)), (json!("b"), json!(3.5))]
    );
    let run = rows.run.expect("the run");
    assert_eq!(
        (run.goal, run.outcome, run.deadline),
        (json!("ship"), json!(null), json!(5.0))
    );
    assert_eq!(rows.revision, 1, "the latest event");
}
