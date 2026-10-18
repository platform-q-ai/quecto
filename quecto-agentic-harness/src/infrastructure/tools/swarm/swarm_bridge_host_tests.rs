//! The host's read of a board (#2278): the Rust board's alone, with no
//! Python and no process started.
use super::{context, create};
use serde_json::json;

/// The page faults of this process's waited-for children, from
/// `/proc/self/stat` (`cminflt` and `cmajflt`): every child that ran and
/// was reaped adds to them (its exec alone faults its pages in), and
/// nothing else does.
fn reaped_children_faults() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    // The fields after the command name, which ends at the last `)`: the
    // state is field 3, cminflt field 11 and cmajflt field 13.
    let fields: Vec<&str> = stat[stat.rfind(')').unwrap() + 1..]
        .split_whitespace()
        .collect();
    fields[8].parse::<u64>().unwrap() + fields[10].parse::<u64>().unwrap()
}

/// This process's children not yet reaped, running or exited, from every
/// thread's `/proc/self/task/<tid>/children`.
fn unreaped_children() -> String {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .map(|task| std::fs::read_to_string(task.unwrap().path().join("children")).unwrap())
        .collect()
}

/// The host reads a board with no Python (#2278): in a child process
/// whose `PATH` is an empty directory, where no interpreter can be found
/// (review M3), the read still succeeds, and it starts no process at all
/// (final review nit): no child of the reading process ran and was
/// reaped, and none is left running or unreaped.
#[test]
fn hosted_run_reads_without_python() {
    const CHILD: &str = "QUECTO_TEST_2278_NO_PYTHON_CHILD";
    if let Ok(checkout) = std::env::var(CHILD) {
        // No program can be found by name, an interpreter included: every
        // `PATH` entry is empty, so the read is the Rust board's alone.
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
        assert!(!dirs.is_empty(), "the child has a PATH");
        for dir in &dirs {
            assert!(
                std::fs::read_dir(dir).unwrap().next().is_none(),
                "{} holds no program",
                dir.display()
            );
        }
        // The measure sees a child that ran: this test binary, listing
        // no test, by its absolute path.
        let before = reaped_children_faults();
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--list", "--exact", "no-such-test"])
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        assert!(
            reaped_children_faults() > before,
            "a reaped child adds its page faults"
        );
        let before = reaped_children_faults();
        let hosted = super::super::swarm_bridge::HostedStore::at(
            checkout.into(),
            crate::composition::swarm::swarm_board(),
        );
        let run = hosted.hosted_run().unwrap().expect("a created run");
        assert_eq!(run.coordinator, "parent");
        assert_eq!(
            reaped_children_faults(),
            before,
            "the read ran no child process"
        );
        assert_eq!(
            unreaped_children().trim(),
            "",
            "the read left no child process"
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    create(&context(&directory), 1);
    let empty = tempfile::tempdir().unwrap();
    let name = std::thread::current().name().unwrap().to_owned();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture", "--test-threads=1"])
        .env(CHILD, directory.path())
        .env("PATH", empty.path())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn hosted(directory: &tempfile::TempDir) -> super::super::swarm_bridge::HostedStore {
    super::super::swarm_bridge::HostedStore::at(
        directory.path().into(),
        crate::composition::swarm::swarm_board(),
    )
}

fn sql(directory: &tempfile::TempDir, statements: &str) {
    let database = directory.path().join(".quecto/swarm.sqlite");
    rusqlite::Connection::open(database)
        .unwrap()
        .execute_batch(statements)
        .unwrap();
}

/// A run with two items: item 1 claimed by `parent` with a reserved path,
/// item 2 waiting on item 1. Its claim and reservation tokens.
fn run_with_items(directory: &tempfile::TempDir) -> [String; 2] {
    let context = context(directory);
    create(&context, 2);
    super::super::super::registry::call_work::off_the_runtime(|| {
        let create = |request: &str, title: &str, depends: serde_json::Value| {
            context
                .call("task_create", json!([request, title, ["passes"], depends]))
                .unwrap()
        };
        create("r1", "Plumbing", json!([]));
        create("r2", "CAS", json!([1]));
        let claimed = context.call("claim", json!([1])).unwrap();
        let reserved = context
            .call("reserve", json!([1, claimed["token"], ["src/a.rs"]]))
            .unwrap();
        [
            claimed["token"].as_str().unwrap().to_owned(),
            reserved["token"].as_str().unwrap().to_owned(),
        ]
    })
}

/// The supervising session reads the whole board (#2478) through the
/// board's own adapter: the run, the items with their reserved paths, the
/// members and totals at the board's revision, with no claim or
/// reservation token and no member reservation, pid, socket or launcher.
#[test]
fn the_host_reads_the_board_projection_without_secrets() {
    let directory = tempfile::tempdir().unwrap();
    let hosted = hosted(&directory);
    assert_eq!(
        hosted.board_projection().unwrap(),
        None,
        "no store, no board"
    );
    assert_eq!(hosted.board_revision().unwrap(), None);
    let tokens = run_with_items(&directory);
    sql(
        &directory,
        "UPDATE members SET pid=4242, socket='secret-socket', launcher='secret-launcher'",
    );
    let board = hosted.board_projection().unwrap().expect("a board");
    let revision = hosted.board_revision().unwrap().expect("a revision");
    assert_eq!(board["revision"].as_i64(), Some(revision.revision));
    assert_eq!(
        board["run"]["id"].as_str(),
        revision.run.as_deref(),
        "L4 (#2487 round 2): the revision names its run"
    );
    assert!(revision.created, "round 3 L3: and says it was created");
    assert_eq!(
        (board["run"]["goal"].clone(), board["run"]["status"].clone()),
        (json!("ship"), json!("running"))
    );
    assert_eq!(
        (
            board["items"][0]["state"].clone(),
            board["items"][0]["owner"].clone()
        ),
        (json!("claimed"), json!("parent"))
    );
    assert_eq!(board["items"][0]["reserved_paths"], json!(["src/a.rs"]));
    assert_eq!(board["items"][1]["state"], json!("ready"));
    assert_eq!(
        (
            board["items"][1]["waiting"].clone(),
            board["items"][1]["depends_on"].clone()
        ),
        (json!(true), json!([1]))
    );
    assert_eq!(
        (
            board["members"][0]["role"].clone(),
            board["members"][0]["claims"].clone()
        ),
        (json!("coordinator"), json!([1]))
    );
    assert_eq!(
        (
            board["totals"]["claimed"].clone(),
            board["totals"]["waiting"].clone()
        ),
        (json!(1), json!(1))
    );
    assert_eq!(board["truncated"], json!(false));
    let text = board.to_string();
    let database = directory.path().join(".quecto/swarm.sqlite");
    let reservation: String = rusqlite::Connection::open(database)
        .unwrap()
        .query_row("SELECT reservation FROM members", [], |row| row.get(0))
        .unwrap();
    for secret in [
        tokens[0].as_str(),
        tokens[1].as_str(),
        reservation.as_str(),
        "4242",
        "secret-",
    ] {
        assert!(!text.contains(secret), "{secret} crossed: {text}");
    }
}

/// H1: recording a request's usage writes no event, yet the projection
/// shows its totals, so it moves the revision too.
#[test]
fn recording_usage_moves_the_revision() {
    let directory = tempfile::tempdir().unwrap();
    run_with_items(&directory);
    let hosted = hosted(&directory);
    let before = hosted.board_revision().unwrap().unwrap().revision;
    let context = context(&directory);
    super::super::super::registry::call_work::off_the_runtime(|| {
        let failed = json!({"request_id": "r9", "instrumented_attempts": 1, "outcome": "failed"});
        context.call("_record_request", json!([failed])).unwrap();
    });
    let after = hosted.board_revision().unwrap().unwrap().revision;
    assert!(after > before, "{before} -> {after}");
    let board = hosted.board_projection().unwrap().unwrap();
    assert_eq!(board["revision"], json!(after));
    assert_eq!(board["run"]["budget"]["unknown_usage_requests"], json!(1));
}

/// H2: a board edited past the byte budget answers what fits and says
/// rows were left out, far inside a socket line.
#[test]
fn a_board_past_the_byte_budget_is_truncated() {
    let directory = tempfile::tempdir().unwrap();
    run_with_items(&directory);
    sql(
        &directory,
        &format!(
            "UPDATE tasks SET title='{}' WHERE id=2",
            "x".repeat(5 * 1024 * 1024)
        ),
    );
    let board = hosted(&directory).board_projection().unwrap().unwrap();
    assert_eq!(board["truncated"], json!(true));
    assert_eq!(board["items"].as_array().unwrap().len(), 1);
    assert_eq!(board["totals"]["items"], json!(2));
    assert!(board.to_string().len() < 4 * 1024 * 1024 + 4096);
}

/// L2 (#2488 review): the budget counts encoded bytes, so a board whose
/// text escapes (a control character is six bytes in JSON) still answers
/// inside the budget, far from the socket's 8 MiB line.
#[test]
fn an_escaping_board_stays_within_the_budget_once_encoded() {
    let directory = tempfile::tempdir().unwrap();
    run_with_items(&directory);
    sql(
        &directory,
        "UPDATE tasks SET title=replace(hex(zeroblob(1572864)), '00', char(1))",
    );
    let board = hosted(&directory).board_projection().unwrap().unwrap();
    assert_eq!(board["truncated"], json!(true));
    let encoded = board.to_string().len();
    assert!(encoded < 4 * 1024 * 1024 + 4096, "{encoded} bytes encoded");
}

/// M4: the projection reads named columns only: a token that is not
/// UTF-8 or a BLOB column a later schema adds never breaks it, and a
/// projected column that is not UTF-8 is refused by name, never echoed.
#[test]
fn only_named_columns_are_read_and_bad_text_is_refused_by_name() {
    let directory = tempfile::tempdir().unwrap();
    run_with_items(&directory);
    sql(
        &directory,
        "UPDATE tasks SET token=CAST(x'ff' AS TEXT); ALTER TABLE tasks ADD COLUMN extra BLOB; UPDATE tasks SET extra=x'00ff'; ALTER TABLE members ADD COLUMN later BLOB; UPDATE members SET later=x'ff'",
    );
    assert!(hosted(&directory).board_projection().unwrap().is_some());
    sql(
        &directory,
        "UPDATE tasks SET title=CAST(x'ff4142' AS TEXT) WHERE id=1",
    );
    let refused = hosted(&directory)
        .board_projection()
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("tasks title holds text that is not UTF-8"),
        "{refused}"
    );
}

/// M3: the projection is a read: it reads a board an older writer laid
/// out (no outcome columns, no usage ledger) without altering it, and it
/// answers while a member holds the write lock.
#[test]
fn the_projection_migrates_nothing_and_reads_beside_a_writer() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let mut legacy = String::new();
    for statement in crate::infrastructure::persistence::swarm_board::schema::schema_statements() {
        legacy.push_str(statement);
        legacy.push(';');
    }
    legacy.push_str("INSERT INTO run VALUES ('run-1','ship','[]','[]','parent','parent',2,9999999999.0,'running');");
    legacy.push_str("INSERT INTO members (id, status) VALUES ('parent','live');");
    sql(&directory, &legacy);
    let schema = |directory: &tempfile::TempDir| -> Vec<String> {
        let database = directory.path().join(".quecto/swarm.sqlite");
        let connection = rusqlite::Connection::open(database).unwrap();
        let mut statement = connection
            .prepare("SELECT name || ':' || coalesce(sql, '') FROM sqlite_master ORDER BY name")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let before = schema(&directory);
    let board = hosted(&directory).board_projection().unwrap().unwrap();
    assert_eq!(
        (
            board["run"]["outcome"].clone(),
            board["run"]["budget"]["observed_tokens"].clone()
        ),
        (json!(null), json!(0))
    );
    assert_eq!(schema(&directory), before, "nothing was created or altered");

    let database = directory.path().join(".quecto/swarm.sqlite");
    let writer = rusqlite::Connection::open(database).unwrap();
    writer
        .execute_batch("BEGIN IMMEDIATE; UPDATE run SET goal='unseen'")
        .unwrap();
    let during = hosted(&directory).board_projection().unwrap().unwrap();
    assert_eq!(
        during["run"]["goal"],
        json!("ship"),
        "the committed board, beside a writer"
    );
    writer.execute_batch("ROLLBACK").unwrap();
}

/// L9, and L2 of #2487 round 2: a board beside one the layout no longer
/// places is refused as the run observation refuses it: only while it
/// holds no created run. One that holds a created run is the board its
/// members run on, and is read.
#[test]
fn a_board_with_a_displaced_one_beside_it_is_refused_without_a_created_run() {
    let directory = tempfile::tempdir().unwrap();
    run_with_items(&directory);
    let git = directory.path().join(".git/quecto");
    std::fs::create_dir_all(&git).unwrap();
    std::fs::copy(
        directory.path().join(".quecto/swarm.sqlite"),
        git.join("swarm.sqlite"),
    )
    .unwrap();
    let board = hosted(&directory).board_projection().unwrap().unwrap();
    assert_eq!(board["run"]["goal"], json!("ship"));
    assert!(hosted(&directory).board_revision().unwrap().is_some());
    // The board the layout now places (in the git directory) is the
    // current one; the copy left in `.quecto` is the displaced one.
    rusqlite::Connection::open(git.join("swarm.sqlite"))
        .unwrap()
        .execute_batch("UPDATE run SET deadline=0")
        .unwrap();
    for refused in [
        hosted(&directory)
            .board_projection()
            .unwrap_err()
            .to_string(),
        hosted(&directory).board_revision().unwrap_err().to_string(),
    ] {
        assert!(
            refused.contains("holds no created run, but another board exists"),
            "{refused}"
        );
    }
}

/// L3 (#2487 round 2): a JSON column the board could not have written is
/// refused by table and column, without its content: a lone surrogate in
/// an item's criteria, or dependencies that are not JSON at all.
#[test]
fn unreadable_json_is_refused_without_its_content() {
    for (edit, column) in [
        (
            r#"UPDATE tasks SET acceptance='["\ud800 secret"]' WHERE id=1"#,
            "tasks acceptance",
        ),
        (
            "UPDATE tasks SET dependencies='[1, secret' WHERE id=2",
            "tasks dependencies",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        run_with_items(&directory);
        sql(&directory, edit);
        let refused = hosted(&directory)
            .board_projection()
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains(&format!("{column} is not JSON this board writes")),
            "{refused}"
        );
        assert!(!refused.contains("secret"), "{refused}");
    }
}

/// Verification item 3 (#2488): the supervising session's reads are an
/// allowlist checked before anything is looked for or called, board or
/// no board.
#[test]
fn only_the_projection_reads_are_supervised() {
    let empty = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    run_with_items(&directory);
    for directory in [&empty, &directory] {
        let refused = hosted(directory)
            .supervised("_status")
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains(
                "the supervising session reads only _board and _board_revision, not _status"
            ),
            "{refused}"
        );
    }
}
