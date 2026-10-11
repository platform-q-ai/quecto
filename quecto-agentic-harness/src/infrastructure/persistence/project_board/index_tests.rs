use super::*;
use crate::application::project_board::ports::{BoardBootstrap, BoardHead, BoardWrite};
use crate::domain::project_board::entities::claim::{Claim, Identity};
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::value_objects::vocabulary::TaskKind;
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use std::process::Command;

/// A history where no snapshot is behind another: every rebuild publishes.
struct Unrelated;

impl BoardRevisions for Unrelated {
    fn remote_head(&self) -> Result<Option<String>, BoardStoreError> {
        Ok(None)
    }

    fn is_ancestor(&self, _: &str, _: &str) -> Result<bool, BoardStoreError> {
        Ok(false)
    }
}

fn rebuild(index: &BoardIndex, snapshot: &BoardSnapshot) -> Result<Refreshed, IndexError> {
    index.rebuild(snapshot, &Unrelated)
}

fn index() -> (tempfile::TempDir, BoardIndex) {
    let dir = tempfile::tempdir().unwrap();
    let index = BoardIndex::at(dir.path().join("quecto").join("project-board.sqlite"));
    (dir, index)
}

/// A connection that can change the index file, as something other than
/// the index might.
fn tamper(index: &BoardIndex, sql: &str) {
    let connection = rusqlite::Connection::open(index.path()).unwrap();
    connection.execute_batch(sql).unwrap();
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn task(id: &str, status: TaskStatus, parent: Option<&str>, depends_on: &[&str]) -> Task {
    let at = Timestamp::parse("2026-10-10T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: slug(id),
        title: format!("Task {id}"),
        kind: TaskKind::Task,
        description: String::new(),
        status,
        parent: parent.map(slug),
        depends_on: depends_on.iter().map(|id| slug(id)).collect(),
        plan: vec![],
        items: vec![],
        team: None,
        claim: None,
        runs: vec![],
        reviews: vec![],
        prs: vec![],
        created: at.clone(),
        updated: at,
    })
    .unwrap()
}

/// a and b ready, c a draft, d done; b and c are a's children; b depends
/// on a (so only a may start), d on a and b; one file set aside.
fn board(head: &str) -> BoardSnapshot {
    BoardSnapshot {
        head: BoardHead(head.repeat(40 / head.len())),
        tasks: vec![
            task("a", TaskStatus::Ready, None, &[]),
            task("b", TaskStatus::Ready, Some("a"), &["a"]),
            task("c", TaskStatus::Draft, Some("a"), &[]),
            task("d", TaskStatus::Done, None, &["a", "b"]),
        ],
        quarantined: vec![
            QuarantinedFile {
                path: "tasks/x.json".into(),
                id: Some(slug("x")),
                reason: "malformed".into(),
            },
            QuarantinedFile {
                path: "tasks/x.txt".into(),
                id: None,
                reason: "not a task file".into(),
            },
        ],
    }
}

/// The same board one commit later: a is gone (so b waits on a task not
/// on the board), e is ready.
fn moved(head: &str) -> BoardSnapshot {
    let mut snapshot = board(head);
    snapshot.tasks.remove(0);
    snapshot.tasks.push(task("e", TaskStatus::Ready, None, &[]));
    snapshot.quarantined.clear();
    snapshot
}

fn ids(listing: Result<BoardListing, IndexError>) -> Vec<String> {
    listing
        .unwrap()
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_string())
        .collect()
}

/// `id` in `status`, held by Ada since 09:00 until `expires` (or not held).
fn held(id: &str, status: TaskStatus, expires: Option<&str>) -> Task {
    let mut fields = task(id, TaskStatus::Ready, None, &[]).into_fields();
    fields.status = status;
    let at = |time: &str| Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap();
    fields.updated = at("09:00:00");
    fields.claim = expires.map(|expires| Claim {
        holder: Identity {
            name: "Ada".into(),
            email: "ada@example.com".into(),
        },
        since: at("09:00:00"),
        expires: at(expires),
    });
    Task::new(fields).unwrap()
}

/// Every case the listings tell apart: dependencies done, archived,
/// missing, unfinished or mixed; claims expired, live or absent in every
/// status that may hold one; children of tasks on the board.
fn listings_board() -> Vec<Task> {
    vec![
        task("epic", TaskStatus::Draft, None, &[]),
        task("r-alone", TaskStatus::Ready, Some("epic"), &[]),
        task("r-after-done", TaskStatus::Ready, Some("epic"), &["done"]),
        task("r-after-archived", TaskStatus::Ready, None, &["archived"]),
        task("r-after-missing", TaskStatus::Ready, None, &["missing"]),
        task("r-after-ready", TaskStatus::Ready, None, &["r-alone"]),
        task(
            "r-after-mixed",
            TaskStatus::Ready,
            None,
            &["done", "w-live"],
        ),
        task("done", TaskStatus::Done, Some("epic"), &[]),
        task("archived", TaskStatus::Archived, None, &[]),
        task("draft", TaskStatus::Draft, Some("r-alone"), &["archived"]),
        held("c-expired", TaskStatus::Claimed, Some("10:00:00")),
        held("w-expired", TaskStatus::InProgress, Some("10:00:00")),
        held("w-live", TaskStatus::InProgress, Some("11:00:00")),
        held("v-expired", TaskStatus::Review, Some("10:00:00")),
        held("v-unheld", TaskStatus::Review, None),
        held("b-expired", TaskStatus::Blocked, Some("10:00:00")),
    ]
}

#[test]
fn ready_needs_every_dependency_released_and_takeable_an_expired_claim() {
    let (_dir, index) = index();
    let snapshot = BoardSnapshot {
        head: BoardHead("c".repeat(40)),
        tasks: listings_board(),
        quarantined: board("a1").quarantined,
    };
    rebuild(&index, &snapshot).unwrap();
    assert_eq!(
        ids(index.ready()),
        ["r-after-archived", "r-after-done", "r-alone"]
    );
    let at = |time: &str| Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap();
    let expired = ["b-expired", "c-expired", "v-expired", "w-expired"];
    assert_eq!(ids(index.takeable(&at("10:30:00"))), expired);
    assert_eq!(
        ids(index.takeable(&at("10:00:00"))),
        expired,
        "a claim ends at its expiry"
    );
    assert_eq!(ids(index.takeable(&at("09:59:59"))), Vec::<String>::new());
    let listing = index.takeable(&at("10:30:00")).unwrap();
    assert_eq!(
        (listing.head, listing.quarantined),
        (snapshot.head, board("a1").quarantined),
        "every listing carries its head and what is set aside"
    );
}

/// The index means what the capability's `QueryBoard` means, listing for
/// listing, on the same board. Quarantine is listed by path on both sides:
/// the index orders it so, and the memory store keeps the order it was
/// set aside in, which is path order here.
#[test]
fn the_index_lists_what_query_board_lists() {
    use crate::application::project_board::use_cases::QueryBoard;
    use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
    let memory = MemoryBoardStore::default();
    memory.bootstrap().unwrap();
    let head = memory.read().unwrap().head;
    let written = memory.write(&head, &listings_board(), "board: seed");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    memory.quarantine("q-first", "malformed");
    memory.quarantine("q-second", "holds another task");
    let query = QueryBoard::new(std::sync::Arc::new(memory.client()));
    let (_dir, index) = index();
    let mut snapshot = memory.read().unwrap();
    snapshot.head = BoardHead("d".repeat(40));
    rebuild(&index, &snapshot).unwrap();
    let names = |listing: crate::application::project_board::dto::BoardListing| -> Vec<String> {
        listing
            .tasks
            .iter()
            .map(|task| task.fields().id.as_str().to_string())
            .collect()
    };
    assert_eq!(ids(index.ready()), names(query.ready().unwrap()));
    for time in ["09:59:59", "10:00:00", "10:30:00", "11:00:00"] {
        let now = Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap();
        assert_eq!(
            ids(index.takeable(&now)),
            names(query.takeable(&now).unwrap()),
            "{time}"
        );
    }
    for status in TaskStatus::ALL.iter().copied() {
        assert_eq!(
            ids(index.by_status(status)),
            names(query.by_status(status).unwrap()),
            "{status:?}"
        );
    }
    for id in ["epic", "r-alone", "done", "missing"] {
        assert_eq!(
            ids(index.children(&slug(id))),
            names(query.children(&slug(id)).unwrap()),
            "{id}"
        );
    }
    for id in ["done", "r-alone", "w-live", "archived", "missing"] {
        assert_eq!(
            ids(index.dependents(&slug(id))),
            names(query.dependents(&slug(id)).unwrap()),
            "{id}"
        );
    }
    let quarantined = index.ready().unwrap().quarantined;
    assert_eq!(quarantined, query.ready().unwrap().quarantined);
    assert_eq!(quarantined.len(), 2);
}

#[test]
fn the_index_answers_ready_status_children_dependents_and_quarantine() {
    let (_dir, index) = index();
    assert_eq!(
        rebuild(&index, &board("a1")),
        Ok(Refreshed::Rebuilt {
            head: board("a1").head.0
        })
    );
    assert_eq!(ids(index.ready()), ["a"]);
    assert_eq!(ids(index.by_status(TaskStatus::Draft)), ["c"]);
    assert_eq!(
        ids(index.by_status(TaskStatus::Archived)),
        Vec::<String>::new()
    );
    assert_eq!(ids(index.children(&slug("a"))), ["b", "c"]);
    assert_eq!(ids(index.dependents(&slug("a"))), ["b", "d"]);
    assert_eq!(ids(index.dependents(&slug("b"))), ["d"]);
    assert_eq!(ids(index.by_status(TaskStatus::Ready)), ["a", "b"]);
    assert_eq!(
        index.by_status(TaskStatus::Done),
        Ok(BoardListing {
            head: board("a1").head,
            tasks: vec![board("a1").tasks[3].clone()],
            quarantined: board("a1").quarantined,
        })
    );
    assert_eq!(index.head(), Ok(Some(board("a1").head.0)));
}

#[test]
fn an_unmoved_board_is_not_rebuilt_and_a_moved_one_replaces_every_row() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    assert_eq!(
        rebuild(&index, &board("a1")),
        Ok(Refreshed::Unchanged {
            head: board("a1").head.0
        })
    );
    assert_eq!(
        rebuild(&index, &moved("b2")),
        Ok(Refreshed::Rebuilt {
            head: moved("b2").head.0
        })
    );
    assert_eq!(ids(index.ready()), ["e"]);
    assert_eq!(
        ids(index.dependents(&slug("a"))),
        ["b", "d"],
        "rows follow the new board"
    );
    assert_eq!(index.ready().unwrap().quarantined, vec![]);
}

#[test]
fn a_snapshot_head_that_is_not_a_full_commit_id_is_refused() {
    let (_dir, index) = index();
    for head in ["r1", "HEAD", &"A".repeat(40), &"a".repeat(39)] {
        let mut snapshot = board("a1");
        snapshot.head = BoardHead(head.to_string());
        assert!(
            matches!(rebuild(&index, &snapshot), Err(IndexError::Corrupt(_))),
            "{head}"
        );
    }
    assert_eq!(index.head(), Err(IndexError::NeedsRebuild));
}

#[test]
fn a_missing_deleted_or_foreign_index_is_rebuilt() {
    let (_dir, index) = index();
    assert_eq!(index.ready(), Err(IndexError::NeedsRebuild));
    rebuild(&index, &board("a1")).unwrap();
    std::fs::remove_file(index.path()).unwrap();
    assert_eq!(index.head(), Err(IndexError::NeedsRebuild));
    assert_eq!(
        rebuild(&index, &board("a1")),
        Ok(Refreshed::Rebuilt {
            head: board("a1").head.0
        })
    );
    std::fs::write(
        index.path(),
        b"not a database, just bytes that are long enough to be read as a header",
    )
    .unwrap();
    assert_eq!(index.ready(), Err(IndexError::NeedsRebuild));
    assert_eq!(
        rebuild(&index, &board("a1")),
        Ok(Refreshed::Rebuilt {
            head: board("a1").head.0
        })
    );
    assert_eq!(ids(index.ready()), ["a"]);
}

#[test]
fn another_layout_or_record_schema_is_rebuilt_even_at_the_same_commit() {
    for sql in [
        "PRAGMA user_version = 99;",
        "UPDATE meta SET value = '0' WHERE key = 'record_schema';",
        "DELETE FROM meta WHERE key = 'record_schema';",
    ] {
        let (_dir, index) = index();
        rebuild(&index, &board("a1")).unwrap();
        tamper(&index, sql);
        assert_eq!(index.ready(), Err(IndexError::NeedsRebuild), "{sql}");
        assert_eq!(
            rebuild(&index, &board("a1")),
            Ok(Refreshed::Rebuilt {
                head: board("a1").head.0
            }),
            "{sql}"
        );
        assert_eq!(ids(index.ready()), ["a"]);
    }
}

#[test]
fn rows_are_only_a_prefilter_and_oversized_values_are_not_read() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    tamper(
        &index,
        "UPDATE tasks SET status = 'ready', parent = 'b' WHERE id = 'c';",
    );
    assert_eq!(
        ids(index.by_status(TaskStatus::Ready)),
        ["a", "b"],
        "c's record says draft"
    );
    assert_eq!(ids(index.children(&slug("b"))), Vec::<String>::new());
    tamper(&index, "UPDATE tasks SET id = 'z' WHERE id = 'a';");
    assert!(matches!(
        index.by_status(TaskStatus::Ready),
        Err(IndexError::Corrupt(_))
    ));
    rebuild(&index, &moved("b2")).unwrap();
    let over = MAX_VALUE_BYTES + 1;
    tamper(
        &index,
        &format!("UPDATE tasks SET record = zeroblob({over}) WHERE id = 'e';"),
    );
    let read = index.by_status(TaskStatus::Ready);
    assert!(
        matches!(&read, Err(IndexError::Corrupt(why)) if why.contains("cap")),
        "{read:?}"
    );
}

#[test]
fn readers_see_one_whole_board_or_the_other_while_it_is_rebuilt() {
    let (_dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let writer = {
        let index = index.clone();
        std::thread::spawn(move || {
            for round in 0..40 {
                let snapshot = if round % 2 == 0 {
                    moved(&format!("{round:02}"))
                } else {
                    board(&format!("{round:02}"))
                };
                rebuild(&index, &snapshot).unwrap();
            }
        })
    };
    let (old, new) = (vec!["a".to_string()], vec!["e".to_string()]);
    let mut reads = 0;
    while !writer.is_finished() || reads < 100 {
        let ready = ids(index.ready());
        assert!(
            ready == old || ready == new,
            "a half-built index was read: {ready:?}"
        );
        reads += 1;
    }
    writer.join().unwrap();
}

/// The child process of [`a_crash_mid_rebuild_leaves_the_last_whole_index`],
/// which starts it with the crash switch on: a real rebuild that dies
/// inside `fill`. Run on its own, the switch is off and the same rebuild
/// completes.
#[test]
fn crash_mid_rebuild_child() {
    let (_dir, scratch) = index();
    let path = std::env::var_os("QUECTO_BOARD_INDEX_CRASH")
        .map_or_else(|| scratch.path().to_path_buf(), std::path::PathBuf::from);
    assert_eq!(
        rebuild(&BoardIndex::at(path), &moved("b2")),
        Ok(Refreshed::Rebuilt {
            head: moved("b2").head.0
        })
    );
}

#[test]
fn a_crash_mid_rebuild_leaves_the_last_whole_index() {
    let (dir, index) = index();
    rebuild(&index, &board("a1")).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "infrastructure::persistence::project_board::index::tests::crash_mid_rebuild_child",
        ])
        .args(["--nocapture", "--test-threads=1"])
        .env("QUECTO_BOARD_INDEX_CRASH", index.path())
        .env(ABORT_IN_FILL, "1")
        .output()
        .unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        output.status.signal(),
        Some(libc::SIGABRT),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(index.head(), Ok(Some(board("a1").head.0)));
    assert_eq!(ids(index.ready()), ["a"]);
    let builds = || -> Vec<std::path::PathBuf> {
        std::fs::read_dir(dir.path().join("quecto"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().ends_with(".build"))
            .collect()
    };
    let left = builds();
    assert_eq!(left.len(), 1, "the dead build's file is left");
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&left[0])
        .unwrap()
        .set_modified(hour_ago)
        .unwrap();
    assert_eq!(
        rebuild(&index, &moved("b2")),
        Ok(Refreshed::Rebuilt {
            head: moved("b2").head.0
        })
    );
    assert_eq!(
        builds(),
        Vec::<std::path::PathBuf>::new(),
        "the next rebuild sweeps it"
    );
}

#[path = "index_git_tests.rs"]
mod git;
#[path = "index_hostile_tests.rs"]
mod hostile;
#[path = "index_lock_tests.rs"]
mod lock;
#[path = "index_side_file_tests.rs"]
mod side_files;
