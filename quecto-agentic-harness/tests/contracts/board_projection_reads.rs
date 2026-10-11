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
    bytes: 1_000_000,
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
    rows_within(repository, LIMITS).unwrap()
}

fn rows_within(
    repository: &SqliteBoardRepository,
    limits: ProjectionLimits,
) -> Result<ProjectionRows, BoardError> {
    let mut rows = None;
    repository.read(&mut |transaction| {
        rows = Some(transaction.projection_rows(limits)?);
        Ok(())
    })?;
    Ok(rows.unwrap())
}

/// Three tasks titled `first`, `second` and a 100-byte `x…`.
fn three_tasks(repository: &SqliteBoardRepository) {
    within(repository, |transaction| {
        for title in ["first".to_owned(), "second".to_owned(), "x".repeat(100)] {
            transaction.insert_task(&NewTask {
                title,
                acceptance: json!(["passes"]),
                dependencies: json!([]),
            })?;
        }
        Ok(())
    });
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

/// M2 (#2487 round 2): rows past the byte budget are never copied out of
/// the store. Each item's stored text is measured before it is read (the
/// first two hold under 32 bytes each, the third over 100), so a 64-byte
/// budget reads the first two and stops; every item's state is still read.
#[test]
fn rows_past_the_byte_budget_are_never_read() {
    let (_dir, database, repository) = board();
    three_tasks(&repository);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("UPDATE tasks SET owner='b', status='claimed' WHERE id=3")
        .unwrap();
    let rows = rows_within(
        &repository,
        ProjectionLimits {
            items: 10,
            bytes: 64,
            ..LIMITS
        },
    )
    .unwrap();
    let titles: Vec<_> = rows.items.iter().map(|item| item.title.clone()).collect();
    assert_eq!(titles, [json!("first"), json!("second")]);
    assert_eq!(rows.item_count, 3);
    let states: Vec<_> = rows
        .states
        .iter()
        .map(|state| (state.id.clone(), state.status.clone(), state.owner.clone()))
        .collect();
    assert_eq!(
        states,
        [
            (json!(1), json!("ready"), json!(null)),
            (json!(2), json!("ready"), json!(null)),
            (json!(3), json!("claimed"), json!("b")),
        ]
    );
}

/// M2 (#2487 round 2): a state cell larger than the board ever writes is
/// refused naming its columns, never read or echoed.
#[test]
fn an_oversized_state_cell_is_refused_without_its_content() {
    let (_dir, database, repository) = board();
    three_tasks(&repository);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "UPDATE tasks SET dependencies='[' || replace(hex(zeroblob(5000)), '00', '7,') || '7]' WHERE id=2",
        )
        .unwrap();
    let refused = rows_within(&repository, LIMITS).unwrap_err();
    let message = refused.message().to_owned();
    assert!(
        message.contains("tasks status, dependencies or owner holds more than this board writes"),
        "{message}"
    );
    assert!(!message.contains("7,7"), "{message}");
}

/// L7 (#2487 round 2): members, files and evidence come in their order
/// whatever order they were written in. (`tasks.id` is the table's
/// INTEGER PRIMARY KEY, its rowid, so items are stored in id order and no
/// write can put them out of it.)
#[test]
fn members_files_and_evidence_come_in_their_order() {
    let (_dir, database, repository) = board();
    three_tasks(&repository);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "INSERT INTO members (id, status) VALUES ('c','live'), ('a','live'), ('b','dead');
             INSERT INTO files (path, task, owner, claim, token) VALUES ('z.rs',1,'a','c','t'), ('a.rs',2,'a','c','t'), ('m.rs',1,'a','c','t');
             INSERT INTO evidence (criterion, artifact, revision, kind, actor, accepted) VALUES ('tests','x','r','command','b',1), ('lint','x','r','command','c',0), ('tests','x','r','command','a',1);",
        )
        .unwrap();
    let rows = rows(&repository);
    let members: Vec<_> = rows
        .members
        .iter()
        .map(|member| member.id.clone())
        .collect();
    assert_eq!(members, [json!("a"), json!("b"), json!("c")]);
    let files: Vec<_> = rows.files.iter().map(|(path, _)| path.clone()).collect();
    assert_eq!(files, [json!("a.rs"), json!("m.rs"), json!("z.rs")]);
    let evidence: Vec<_> = rows
        .evidence
        .iter()
        .map(|row| (row.criterion.clone(), row.actor.clone()))
        .collect();
    assert_eq!(
        evidence,
        [
            (json!("lint"), json!("c")),
            (json!("tests"), json!("a")),
            (json!("tests"), json!("b")),
        ]
    );
}

/// Round 3 M1 (#2487): every cell is measured before it is copied. A
/// scalar cell (an event time, a run's limit or deadline, an evidence
/// flag, a reservation's item) longer than any number the board writes is
/// refused naming its column, unread and unechoed.
#[test]
fn an_oversized_scalar_cell_is_refused_without_its_content() {
    const HUGE: &str = "replace(hex(zeroblob(3000)), '00', '7')";
    let run = |member_limit: &str, deadline: &str| {
        format!(
            "INSERT INTO run (id, goal, criteria, coordinator, integrator, member_limit, deadline, status) VALUES ('run-1','ship','[]','b','b',{member_limit},{deadline},'running');"
        )
    };
    for (edit, column) in [
        (
            format!(
                "INSERT INTO members (id, status) VALUES ('b','live'); INSERT INTO events (actor, time, action, detail) VALUES ('b', {HUGE}, 'claimed', '{{}}');"
            ),
            "events time",
        ),
        (run(HUGE, "5.0"), "run member_limit"),
        (run("2", HUGE), "run deadline"),
        (
            format!(
                "INSERT INTO evidence (criterion, artifact, revision, kind, actor, accepted) VALUES ('tests','x','r','command','b',{HUGE});"
            ),
            "evidence accepted",
        ),
        (
            format!(
                "INSERT INTO files (path, task, owner, claim, token) VALUES ('a.rs',{HUGE},'b','c','t');"
            ),
            "files task",
        ),
    ] {
        let (_dir, database, repository) = board();
        three_tasks(&repository);
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch(&edit)
            .unwrap();
        let message = rows_within(&repository, LIMITS)
            .unwrap_err()
            .message()
            .to_owned();
        assert!(
            message.contains(&format!("{column} holds more than this board writes")),
            "{column}: {message}"
        );
        assert!(!message.contains("7777"), "{column}: {message}");
    }
}

/// Round 3 L2 (#2487): a member's last activity is the time of its latest
/// board event (the highest id), not the largest time it ever wrote.
#[test]
fn last_activity_is_the_latest_event_s_time() {
    let (_dir, database, repository) = board();
    within(&repository, |_| Ok(()));
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(
            "INSERT INTO members (id, status) VALUES ('b','live');
             INSERT INTO events (actor, time, action, detail) VALUES ('b', 9.0, 'claimed', '{}'), ('b', 3.5, 'released', '{}');",
        )
        .unwrap();
    assert_eq!(rows(&repository).members[0].last_activity, json!(3.5));
}

/// Round 3 L4 (#2487): items are measured before reservations, so long
/// reserved paths never starve the items they belong to.
#[test]
fn items_are_measured_before_reservations() {
    let (_dir, database, repository) = board();
    three_tasks(&repository);
    let path = "p".repeat(40);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch(&format!(
            "INSERT INTO files (path, task, owner, claim, token) VALUES ('a{path}',1,'b','c','t'), ('b{path}',2,'b','c','t');"
        ))
        .unwrap();
    let rows = rows_within(
        &repository,
        ProjectionLimits {
            items: 10,
            bytes: 64,
            ..LIMITS
        },
    )
    .unwrap();
    assert_eq!(rows.items.len(), 2, "the first two items fit first");
    assert!(rows.files.is_empty(), "no reservation fits after them");
}
