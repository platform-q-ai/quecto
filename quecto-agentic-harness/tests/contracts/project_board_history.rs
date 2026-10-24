//! Scoped A history contracts; original B tests deferred intact externally.
use super::project_board_history_fixture::*;
use super::project_board_store::task;
use quecto::application::project_board::ports::{history::*, *};
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
use std::process::Command;

#[test]
fn fixture_control_native_every_kind_unchanged_and_actual_clear() {
    use quecto::infrastructure::test_support::board_artifacts;
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let store = native(&fixture);
    let values = vec![
        BoardArtifact::Task(task("a", "Native task")),
        BoardArtifact::Plan(board_artifacts::plan("a", 1)),
        BoardArtifact::Run(board_artifacts::run("a", 1)),
        BoardArtifact::Change(board_artifacts::change("a", 1)),
        BoardArtifact::Review(board_artifacts::review("a", 1)),
        BoardArtifact::Discussion(board_artifacts::discussion("store-locking")),
    ];
    let head = store.read().unwrap().head;
    let first = written(store.write(&head, &values, "board: all kinds"));
    let second = written(store.write(&first, &values, "board: unchanged all kinds"));
    assert_ne!(first, second);
    let snapshot = store.read().unwrap();
    assert_eq!(
        [
            snapshot.tasks.len(),
            snapshot.plans.len(),
            snapshot.runs.len(),
            snapshot.changes.len(),
            snapshot.reviews.len(),
            snapshot.discussions.len()
        ],
        [1; 6]
    );
    for artifact in &values {
        assert!(
            !quecto::infrastructure::persistence::project_board::artifacts::file_of(artifact)
                .unwrap()
                .is_empty()
        );
    }
    store.fixture_put_raw("tasks/broken.md", b"{").unwrap();
    let snapshot = store.read().unwrap();
    assert_eq!(snapshot.quarantined.len(), 1);
    let q = &snapshot.quarantined[0];
    let clear = SetAsideClear {
        path: q.path.clone(),
        object: q.object.clone(),
        repeated: Default::default(),
    };
    let last = written(store.clear_set_aside(&snapshot.head, &clear, "board: native clear"));
    assert_eq!(store.read().unwrap().head, last);
    assert!(store.read().unwrap().quarantined.is_empty());
    let count: i64 = rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .query_row("SELECT count(*) FROM revisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 5);
    println!("native-control six kinds, unchanged distinct child, actual clear, revisions={count}");
}

fn native(fixture: &Fixture) -> SqliteProjectBoardStore {
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    store.bootstrap().unwrap();
    store
}

fn written(result: Result<BoardWrite, BoardStoreError>) -> BoardHead {
    match result.unwrap() {
        BoardWrite::Written(head) => head,
        other => panic!("expected valid write: {other:?}"),
    }
}

#[test]
fn feature_red_pinned_snapshot_survives_native_write_and_epoch_frontier() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let store = native(&fixture);
    let initial = store.read().unwrap().head;
    let first = written(store.write_tasks(
        &initial,
        &[task("a", "Before snapshot")],
        "board: snapshot before",
    ));
    let handles = fixture.handles();
    let snapshot = handles
        .store
        .snapshot()
        .expect("read-only owned pinned snapshot");
    assert_eq!(snapshot.frontier().unwrap(), first);
    let second = written(store.write_tasks(
        &first,
        &[task("a", "After snapshot")],
        "board: snapshot after",
    ));
    assert_ne!(first, second);
    let promoted = fixture
        .epoch_store()
        .open_epoch(1, "held frontier", 1700000000)
        .unwrap();
    assert_eq!(promoted, BoardHead("2:3".into()));
    assert_eq!(snapshot.frontier().unwrap(), first);
    assert_eq!(
        snapshot.next_revision(1).unwrap().unwrap().message,
        "board: snapshot before"
    );
    assert!(
        snapshot.next_revision(2).unwrap().is_none(),
        "cursor cannot cross captured revision frontier"
    );
    assert_eq!(
        handles.store.snapshot().unwrap().frontier().unwrap(),
        promoted
    );
}

#[test]
fn feature_red_epoch_classifier_inclusive_frontier_known_intervals_and_unseen_old_fence() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let store = native(&fixture);
    let h = store.read().unwrap().head;
    let h = written(store.write_tasks(&h, &[task("a", "One")], "board: one"));
    let h = written(store.write_tasks(&h, &[task("a", "Two")], "board: two"));
    let promoted = fixture
        .epoch_store()
        .open_epoch(1, "minimum epoch primitive", 1700000000)
        .expect("B3 opens epoch at inclusive held frontier, without creating revision");
    assert_eq!(promoted, BoardHead("2:3".into()));
    assert_eq!(store.read().unwrap().head, promoted);
    for token in ["1:1", "1:2", "1:3", "1:300"] {
        let before = fingerprint(&fixture.database);
        assert_eq!(
            store.write_tasks(
                &BoardHead(token.into()),
                &[task("a", "Stale")],
                "board: old"
            ),
            Ok(BoardWrite::Lost),
            "{token}"
        );
        assert_eq!(fingerprint(&fixture.database), before);
    }
    let next = written(store.write_tasks(
        &promoted,
        &[task("a", "Written at held frontier")],
        "board: frontier accepted",
    ));
    assert_eq!(next, BoardHead("2:4".into()));
    {
        let token = "2:3";
        assert_eq!(
            store.write_tasks(
                &BoardHead(token.into()),
                &[task("a", "Stale current")],
                "board: stale"
            ),
            Ok(BoardWrite::Lost)
        );
    }
    for token in [
        "1:4",
        "2:1",
        "2:9999",
        "3:4",
        "999:4",
        "0:4",
        "2:0",
        "-1:4",
        "2:-1",
        "02:4",
        "2:04",
        "2:18446744073709551616",
        "2:9223372036854775808",
        "garbage",
        "2:4:5",
        ":4",
        "2:",
    ] {
        let before = fingerprint(&fixture.database);
        let result = store.write_tasks(
            &BoardHead(token.into()),
            &[task("a", "Invalid")],
            "board: invalid token",
        );
        assert!(
            matches!(result, Err(BoardStoreError::InvalidWrite(_))),
            "{token}: {result:?}"
        );
        assert_eq!(fingerprint(&fixture.database), before);
    }
    assert_ne!(h, next);
    let before = fingerprint(&fixture.database);
    assert!(matches!(
        fixture
            .epoch_store()
            .open_epoch(1, "stale expectation", 1700000000),
        Err(HistoryError::Conflict(_))
    ));
    assert_eq!(fingerprint(&fixture.database), before);
    let promoted_again = fixture
        .epoch_store()
        .open_epoch(2, "second interval", 1700000001)
        .unwrap();
    assert_eq!(promoted_again, BoardHead("3:4".into()));
    let before = fingerprint(&fixture.database);
    assert!(matches!(
        store.write_tasks(
            &BoardHead("2:2".into()),
            &[task("a", "Before retired start")],
            "board: impossible old pair"
        ),
        Err(BoardStoreError::InvalidWrite(_))
    ));
    assert_eq!(
        store.write_tasks(
            &BoardHead("2:300".into()),
            &[task("a", "Unseen old")],
            "board: fence only"
        ),
        Ok(BoardWrite::Lost)
    );
    assert_eq!(
        fingerprint(&fixture.database),
        before,
        "unseen old fence is never claimed as a held/exportable row"
    );
    assert_eq!(
        written(store.write_tasks(
            &promoted_again,
            &[task("a", "Second inclusive frontier")],
            "board: inclusive again"
        )),
        BoardHead("3:5".into())
    );
}

#[test]
fn feature_red_export_over_4096_native_revisions_and_oid_cache_eviction_has_no_truncation() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let store = native(&fixture);
    let mut head = store.read().unwrap().head;
    for index in 0..4100 {
        head = written(store.write_tasks(
            &head,
            &[task("a", &format!("Unique cache object {index}"))],
            "board: long native history",
        ));
    }
    head = written(store.write_tasks(
        &head,
        &[task("a", "Unique cache object 0")],
        "board: revisit evicted early object",
    ));
    let before = fingerprint(&fixture.database);
    let handles = fixture.handles();
    let report = handles
        .export
        .execute(false, None)
        .expect("native export is not capped by import4096 or cache4096");
    assert_eq!((report.revisions, report.frontier), (4102, head));
    let ids = text(&fixture.destination, &["rev-list", "--reverse", TARGET]);
    assert_eq!(ids.lines().count(), 4102);
    let first = ids.lines().next().unwrap();
    let last = ids.lines().last().unwrap();
    assert_ne!(first, last);
    assert_eq!(last, report.git_head);
    let early = ids.lines().nth(1).unwrap();
    assert_ne!(early, last);
    assert_eq!(
        text(
            &fixture.destination,
            &["rev-parse", &format!("{early}^{{tree}}")]
        ),
        text(
            &fixture.destination,
            &["rev-parse", &format!("{last}^{{tree}}")]
        ),
        "restored early content must recompute an evicted tree mapping in the SAME export"
    );
    assert_eq!(
        git(
            &fixture.destination,
            &["show", &format!("{last}:tasks/a.md")],
            b""
        ),
        quecto::infrastructure::persistence::project_board::artifacts::file_of(
            &BoardArtifact::Task(task("a", "Unique cache object 0"))
        )
        .unwrap()
    );
    assert_eq!(
        text(&fixture.destination, &["rev-parse", &format!("{last}^")]),
        ids.lines().nth(4100).unwrap()
    );
    let repeated = handles.export.execute(true, Some(last)).unwrap();
    assert_eq!(
        repeated.git_head, report.git_head,
        "eviction only recomputes, never changes Git IDs"
    );
    assert_eq!(fingerprint(&fixture.database), before);
}

fn structural_tree_collision_contract(corrupt: bool) {
    use quecto::infrastructure::persistence::sqlite::project_board::RawEntry;
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let store = native(&fixture);
    let artifact = BoardArtifact::Task(task("a", "Tree collision"));
    let bytes =
        quecto::infrastructure::persistence::project_board::artifacts::file_of(&artifact).unwrap();
    let mut expected_tree = String::new();
    store
        .fixture_edit(|editor| {
            let root = editor.current()?.1;
            let blob = editor.blob(&bytes)?;
            expected_tree = editor.tree(&[RawEntry {
                mode: "100644".into(),
                name: b"a.md".to_vec(),
                oid: blob,
            }])?;
            Ok(root)
        })
        .unwrap();
    let healthy = store.read().unwrap();
    assert!(
        healthy.tasks.is_empty() && healthy.quarantined.is_empty(),
        "prospective tree is unreachable, current board remains readable"
    );
    let c = rusqlite::Connection::open(&fixture.database).unwrap();
    let (kind, size): (String, i64) = c
        .query_row(
            "SELECT kind,tree_size FROM objects WHERE oid=?1",
            [&expected_tree],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "tree");
    let original: Vec<u8> = c
        .query_row(
            "SELECT raw_name FROM tree_entries WHERE tree=?1 AND ordinal=0",
            [&expected_tree],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, b"a.md");
    if corrupt {
        assert_eq!(
            c.execute(
                "UPDATE tree_entries SET raw_name=?1 WHERE tree=?2 AND ordinal=0",
                rusqlite::params![b"b.md".as_slice(), &expected_tree]
            )
            .unwrap(),
            1
        );
        let mutated: Vec<u8> = c
            .query_row(
                "SELECT raw_name FROM tree_entries WHERE tree=?1 AND ordinal=0",
                [&expected_tree],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original.len(), mutated.len());
        assert_ne!(original, mutated);
        let held_size: i64 = c
            .query_row(
                "SELECT tree_size FROM objects WHERE oid=?1",
                [&expected_tree],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(size, held_size);
    }
    c.close().unwrap();
    assert_eq!(
        store.read().unwrap(),
        healthy,
        "mutation is unreachable and structurally SQL-legal"
    );
    let before = fingerprint(&fixture.database);
    let result = store.write(
        &healthy.head,
        std::slice::from_ref(&artifact),
        "board: reuse expected tree",
    );
    if corrupt {
        assert!(
            matches!(
                result,
                Err(BoardStoreError::Clash(_)
                    | BoardStoreError::InvalidWrite(_)
                    | BoardStoreError::Unavailable(_))
            ),
            "same tree OID and size but differing ordered row bytes must refuse, not publish: {result:?}"
        );
        assert_eq!(
            fingerprint(&fixture.database),
            before,
            "all tables/epochs/provenance/unreachable rows unchanged on tree collision"
        );
        assert_eq!(store.read().unwrap(), healthy);
    } else {
        written(result);
        assert_eq!(
            store.read().unwrap().tasks,
            vec![task("a", "Tree collision")],
            "exact same ordered tree rows deduplicate successfully"
        );
    }
}

#[test]
fn feature_red_same_tree_oid_and_size_but_different_rows_refuses_all_row_mutation() {
    structural_tree_collision_contract(true);
}

#[test]
fn fixture_control_exact_same_tree_rows_are_successful_dedup() {
    structural_tree_collision_contract(false);
}

fn object_inventory(repo: &std::path::Path) -> std::collections::BTreeSet<String> {
    text(
        repo,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

#[test]
fn feature_red_sink_exact_raw_objects_and_dryrun_tree_commit_are_real_git() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let handles = fixture.handles();
    let object = fixture
        .history
        .objects
        .iter()
        .find(|o| o.kind == HistoryKind::Blob)
        .unwrap();
    let before = object_inventory(&fixture.destination);
    assert!(before.is_empty());
    let request = format!("{}\n", object.oid);
    let missing = git(
        &fixture.destination,
        &["cat-file", "--batch-check"],
        request.as_bytes(),
    );
    assert_eq!(missing, format!("{} missing\n", object.oid).as_bytes());
    let id = handles
        .sink
        .raw(object.kind, &object.bytes, Some(&object.oid), true)
        .expect("pure dry-run hashes original exact bytes");
    assert_eq!(id, object.oid);
    assert_eq!(
        git(
            &fixture.destination,
            &["cat-file", "--batch-check"],
            request.as_bytes()
        ),
        missing,
        "dry-run raw must leave absent object absent"
    );
    assert_eq!(
        object_inventory(&fixture.destination),
        before,
        "pure dry-run must not write unreachable objects either"
    );
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
    let id = handles
        .sink
        .raw(object.kind, &object.bytes, Some(&object.oid), false)
        .unwrap();
    assert_eq!(
        git(&fixture.destination, &["cat-file", "blob", &id], b""),
        object.bytes
    );
    let entries = vec![HistoryEntry {
        mode: "100644".into(),
        name: b"a.bin".to_vec(),
        oid: id,
    }];
    let expected_tree = raw_tree(
        &fixture.repo,
        &[("100644", b"a.bin", entries[0].oid.as_str())],
    );
    let before = object_inventory(&fixture.destination);
    let dry_tree = handles.sink.tree(&entries, true).unwrap();
    assert_eq!(dry_tree, expected_tree);
    assert_eq!(
        object_inventory(&fixture.destination),
        before,
        "tree dry-run cannot hash-object -w"
    );
    assert_eq!(
        git(
            &fixture.destination,
            &["cat-file", "--batch-check"],
            format!("{expected_tree}\n").as_bytes()
        ),
        format!("{expected_tree} missing\n").as_bytes()
    );
    let tree = handles.sink.tree(&entries, false).unwrap();
    assert_eq!(tree, expected_tree);
    let expected_body = format!(
        "tree {tree}\nauthor History <history@example.com> 1700000100 +0000\ncommitter History <history@example.com> 1700000100 +0000\n\nnative deterministic\n"
    );
    let expected_commit = hash(&fixture.repo, "commit", expected_body.as_bytes());
    let before = object_inventory(&fixture.destination);
    let dry_commit = handles
        .sink
        .commit(
            HistoryCommit {
                tree: &tree,
                parent: None,
                writer: &writer(),
                message: "native deterministic",
                at: 1700000100,
            },
            true,
        )
        .unwrap();
    assert_eq!(dry_commit, expected_commit);
    assert_eq!(
        object_inventory(&fixture.destination),
        before,
        "commit dry-run cannot write objects"
    );
    assert_eq!(
        git(
            &fixture.destination,
            &["cat-file", "--batch-check"],
            format!("{expected_commit}\n").as_bytes()
        ),
        format!("{expected_commit} missing\n").as_bytes()
    );
    let commit = handles
        .sink
        .commit(
            HistoryCommit {
                tree: &tree,
                parent: None,
                writer: &writer(),
                message: "native deterministic",
                at: 1700000100,
            },
            false,
        )
        .unwrap();
    let raw = git(&fixture.destination, &["cat-file", "commit", &commit], b"");
    assert_eq!(commit, expected_commit);
    assert_eq!(raw, expected_body.as_bytes());
    assert!(raw.starts_with(format!("tree {tree}\n").as_bytes()));
    let expected=b"author History <history@example.com> 1700000100 +0000\ncommitter History <history@example.com> 1700000100 +0000\n";
    assert!(
        raw.windows(expected.len()).any(|bytes| bytes == expected),
        "exact supplied writer/time/UTC fields, not fixed arbitrary commit metadata"
    );
    assert!(raw.ends_with(b"\n\nnative deterministic\n"));
    assert_eq!(
        handles
            .sink
            .commit(
                HistoryCommit {
                    tree: &tree,
                    parent: None,
                    writer: &writer(),
                    message: "native deterministic",
                    at: 1700000100
                },
                true
            )
            .unwrap(),
        commit
    );
    assert!(
        text(&fixture.destination, &["for-each-ref"]).is_empty(),
        "objects verified before any publication"
    );
    handles.sink.publish(None, &commit).unwrap();
    assert_eq!(text(&fixture.destination, &["rev-parse", TARGET]), commit);
}

#[test]
fn fixture_control_operator_serve_parse_is_preserved_without_starting_a_server() {
    let args = vec![
        "serve".into(),
        "--socket".into(),
        "/scratch/not-started.sock".into(),
        "--repo".into(),
        "/scratch/clone".into(),
    ];
    let options = quecto::interface::cli::board_server::serve::parse(&args).unwrap();
    assert_eq!(options.repo, std::path::PathBuf::from("/scratch/clone"));
    assert_eq!(options.remote, "origin");
}

#[test]
fn feature_red_new_offline_cli_export_native_dryrun_without_server() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let home = fixture.directory.path().join("export-home");
    let data = home.join(".local/share");
    let boards = data.join("quecto/boards");
    std::fs::create_dir_all(&boards).unwrap();
    let db = boards.join("0123456789abcdef.sqlite");
    let store = SqliteProjectBoardStore::open(&db, writer()).unwrap();
    store.bootstrap().unwrap();
    let head = store.read().unwrap().head;
    written(store.write_tasks(
        &head,
        &[task("a", "CLI native rollback")],
        "board: CLI export",
    ));
    let before = fingerprint(&db);
    let output = Command::new("timeout")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .args([
            "-s",
            "KILL",
            "60",
            env!("CARGO_BIN_EXE_quecto"),
            "board-server",
            "export",
            "--repo",
            fixture.destination.to_str().unwrap(),
            "--board-id",
            "0123456789abcdef",
            "--to-branch",
            "quecto/rollback",
            "--format",
            "3",
            "--dry-run",
        ])
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &data)
        .env("QUECTO_BASE_DIR", &home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "NEW export feature absent (narrow command RED, not deep semantics): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["revisions"], 2);
    assert_eq!(report["git_head"].as_str().unwrap().len(), 64);
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
    assert_eq!(fingerprint(&db), before);
}

#[test]
fn feature_green_sink_operational_failure_is_not_conflict_and_directory_sort_cannot_hide_duplicates()
 {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let sink = fixture.handles().sink;
    let tree = sink.tree(&[], false).unwrap();
    let head = sink
        .commit(
            HistoryCommit {
                tree: &tree,
                parent: None,
                writer: &writer(),
                message: "bounded publication",
                at: 1700000100,
            },
            false,
        )
        .unwrap();
    let lock = fixture.destination.join("refs/heads/quecto/rollback.lock");
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    std::fs::write(&lock, b"scratch held lock").unwrap();
    let result = sink.publish(None, &head);
    std::fs::remove_file(lock).unwrap();
    assert!(
        matches!(result, Err(HistoryError::Unavailable(_))),
        "unchanged absent lease plus failed lock acquisition is operational, not conflict: {result:?}"
    );
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
    let oid = "b".repeat(40);
    let entries = [
        HistoryEntry {
            mode: "100644".into(),
            name: b"a".to_vec(),
            oid: oid.clone(),
        },
        HistoryEntry {
            mode: "100644".into(),
            name: b"a.c".to_vec(),
            oid: oid.clone(),
        },
        HistoryEntry {
            mode: "040000".into(),
            name: b"a".to_vec(),
            oid,
        },
    ];
    assert!(
        matches!(sink.tree(&entries, true), Err(HistoryError::Malformed(_))),
        "same raw name is invalid even when directory sorting separates its copies"
    );
}
