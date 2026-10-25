//! Scoped A history contracts; original B tests deferred intact externally.
use super::project_board_history_fixture::*;
use super::project_board_store::task;
use quecto::application::project_board::ports::{history::*, *};
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
use std::{collections::BTreeMap, process::Command};

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
    assert_eq!(id, object.oid);
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
    assert_eq!(commit, expected_commit);
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
    // Written objects are stored, exactly, by the time the ref moves (#2748).
    assert_eq!(
        git(
            &fixture.destination,
            &["cat-file", "blob", &object.oid],
            b""
        ),
        object.bytes
    );
    assert_eq!(
        text(
            &fixture.destination,
            &["rev-parse", &format!("{commit}^{{tree}}")]
        ),
        tree
    );
    let raw = git(&fixture.destination, &["cat-file", "commit", &commit], b"");
    assert_eq!(raw, expected_body.as_bytes());
    assert!(raw.starts_with(format!("tree {tree}\n").as_bytes()));
    let expected=b"author History <history@example.com> 1700000100 +0000\ncommitter History <history@example.com> 1700000100 +0000\n";
    assert!(
        raw.windows(expected.len()).any(|bytes| bytes == expected),
        "exact supplied writer/time/UTC fields, not fixed arbitrary commit metadata"
    );
    assert!(raw.ends_with(b"\n\nnative deterministic\n"));
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
    let boards = home.join("board-server/0123456789abcdef");
    std::fs::create_dir_all(&boards).unwrap();
    let db = boards.join("board.sqlite3");
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

// Original B assertions carried from immutable 52dc; no behavior implementation.
fn source_contract(algorithm: HistoryAlgorithm, count: usize) {
    let fixture = Fixture::new(algorithm, count);
    fixture.verify();
    let before = text(&fixture.repo, &["show-ref"]);
    let actual = fixture
        .b_handles()
        .source
        .capture()
        .expect("B3 captures complete supported local history");
    assert_eq!(
        (
            actual.algorithm,
            actual.source_ref.as_str(),
            actual.source_remote.as_str(),
            actual.source_head.as_str()
        ),
        (
            algorithm,
            BOARD,
            "origin",
            fixture.history.source_head.as_str()
        )
    );
    assert_eq!(actual.revisions.len(), count);
    for (actual, expected) in actual.revisions.iter().zip(&fixture.history.revisions) {
        assert_eq!(
            (
                actual.sequence,
                actual.parent,
                &actual.root,
                &actual.original
            ),
            (
                expected.sequence,
                expected.parent,
                &expected.root,
                &expected.original
            ),
            "raw signed/extra headers and each original ID are exact"
        );
    }
    let objects = |history: &LinearHistory| {
        history
            .objects
            .iter()
            .map(|o| (o.oid.clone(), o.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(
        objects(&actual),
        objects(&fixture.history),
        "complete actual-kind closure including odd opaque/archive trees, duplicate names and foreign gitlink exemption"
    );
    assert_eq!(
        text(&fixture.repo, &["show-ref"]),
        before,
        "capture does not fetch/move source refs"
    );
}
fn original_token_contract(algorithm: HistoryAlgorithm, repeated: bool) {
    let fixture = Fixture::new(algorithm, 12);
    fixture.verify();
    let prefix = if repeated {
        fixture.repeated_head.clone()
    } else {
        fixture.set_aside_head.clone()
    };
    git(
        &fixture.repo,
        &["update-ref", BOARD, &prefix, &fixture.history.source_head],
        b"",
    );
    let expected = fixture.current_git_snapshot();
    assert_eq!(expected.head, BoardHead(prefix.clone()));
    assert_eq!(expected.quarantined.len(), if repeated { 3 } else { 1 });
    let report = fixture
        .b_handles()
        .import
        .execute()
        .expect("import historical prefix with original exposed quarantine/seen/keep tokens");
    assert_eq!(report.source_head, prefix);
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    let imported = store.read().unwrap();
    let tokens = |snapshot: &BoardSnapshot| {
        snapshot
            .quarantined
            .iter()
            .map(|q| {
                (
                    (q.path.clone(), q.object.clone()),
                    (q.key.clone(), q.carried),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(
        tokens(&imported),
        tokens(&expected),
        "raw rollback ledger alone is insufficient: SQL-exposed tokens remain original Git OIDs"
    );
    assert_eq!(imported.tasks, expected.tasks);
    let q = expected
        .quarantined
        .iter()
        .find(|q| q.path == if repeated { "tasks/z.md" } else { "tasks/b.md" })
        .unwrap();
    let mut clear = SetAsideClear {
        path: q.path.clone(),
        object: q.object.clone(),
        repeated: Default::default(),
    };
    if repeated {
        let before = fingerprint(&fixture.database);
        clear.repeated.insert(
            "tasks/a.md".into(),
            KeepCopy::Object {
                keep: fixture.repeated_keep.clone(),
                seen: std::collections::BTreeSet::from([fixture.repeated_keep.clone()]),
            },
        );
        assert!(matches!(
            store.clear_set_aside(&imported.head, &clear, "board: incomplete original seen"),
            Ok(BoardWrite::Refused(_))
        ));
        assert_eq!(fingerprint(&fixture.database), before);
        clear.repeated.insert(
            "tasks/a.md".into(),
            KeepCopy::Object {
                keep: fixture.repeated_keep.clone(),
                seen: fixture.repeated_seen.clone(),
            },
        );
    }
    let next = written(store.clear_set_aside(
        &imported.head,
        &clear,
        "board: exact imported original tokens",
    ));
    let after = store.read().unwrap();
    assert_eq!(after.head, next);
    assert!(after.quarantined.is_empty());
    assert_eq!(
        after.tasks,
        vec![task("a", "First")],
        "exact original keep token retains the intended canonical copy"
    );
    let exported = fixture.b_handles().export.execute(false, None).unwrap();
    assert_eq!(
        text(
            &fixture.destination,
            &["rev-parse", &format!("{}^", exported.git_head)]
        ),
        prefix,
        "first native clear child parents captured original H0"
    );
}
#[test]
fn feature_red_original_tokens_sha1_set_aside() {
    original_token_contract(HistoryAlgorithm::Sha1, false);
}
#[test]
fn feature_red_original_tokens_sha256_set_aside() {
    original_token_contract(HistoryAlgorithm::Sha256, false);
}
#[test]
fn feature_red_original_tokens_sha1_repeated_seen_keep() {
    original_token_contract(HistoryAlgorithm::Sha1, true);
}
#[test]
fn feature_red_original_tokens_sha256_repeated_seen_keep() {
    original_token_contract(HistoryAlgorithm::Sha256, true);
}

macro_rules! corpus {
    ($control:ident,$source:ident,$transfer:ident,$algorithm:expr,$count:expr) => {
        #[test]
        fn $control() {
            let fixture = Fixture::new($algorithm, $count);
            fixture.verify();
            assert_eq!(fixture.history.revisions.len(), $count);
            let hex=|bytes:&[u8]|bytes.iter().map(|b|format!("{b:02x}")).collect::<String>();
            for object in &fixture.history.objects {println!("manifest-object algorithm={:?} oid={} kind={:?} length={} raw_hex={} entries={:?}",$algorithm,object.oid,object.kind,object.bytes.len(),hex(&object.bytes),object.entries);}
            for revision in &fixture.history.revisions {let object=revision.original.as_ref().unwrap();println!("manifest-commit algorithm={:?} sequence={} parent={:?} root={} oid={} length={} raw_hex={}",$algorithm,revision.sequence,revision.parent,revision.root,object.oid,object.bytes.len(),hex(&object.bytes));}
            println!(
                "fixture-control {:?} commits={} H0={} raw_objects={} actual-clear={}",
                $algorithm,
                $count,
                fixture.history.source_head,
                fixture.history.objects.len(),
                fixture.clear_head
            );
        }
        #[test]
        fn $source() {
            source_contract($algorithm, $count);
        }
        #[test]
        fn $transfer() {
            transfer_contract($algorithm, $count);
        }
    };
}
corpus!(
    fixture_control_sha1_owner12,
    feature_red_source_sha1_owner12,
    feature_red_transfer_sha1_owner12,
    HistoryAlgorithm::Sha1,
    12
);
corpus!(
    fixture_control_sha256_owner12,
    feature_red_source_sha256_owner12,
    feature_red_transfer_sha256_owner12,
    HistoryAlgorithm::Sha256,
    12
);
corpus!(
    fixture_control_sha1_corpus106,
    feature_red_source_sha1_corpus106,
    feature_red_transfer_sha1_corpus106,
    HistoryAlgorithm::Sha1,
    106
);
corpus!(
    fixture_control_sha256_corpus106,
    feature_red_source_sha256_corpus106,
    feature_red_transfer_sha256_corpus106,
    HistoryAlgorithm::Sha256,
    106
);

fn transfer_contract(algorithm: HistoryAlgorithm, count: usize) {
    let fixture = Fixture::new(algorithm, count);
    fixture.verify();
    let handles = fixture.b_handles();
    let report = handles
        .import
        .execute()
        .expect("B3 imports all originals only after self-export proof");
    assert_eq!(
        (report.revisions, report.source_head.as_str(), report.head),
        (
            count as u64,
            fixture.history.source_head.as_str(),
            BoardHead(format!("1:{count}"))
        )
    );
    let native = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    let snapshot = native.read().unwrap();
    assert_eq!(snapshot.head, BoardHead(format!("1:{count}")));
    assert!(snapshot.quarantined.is_empty());
    assert!(
        snapshot
            .tasks
            .iter()
            .all(|task| task.fields().id.as_str() != "b"),
        "actual imported clear remains cleared"
    );
    let before = fingerprint(&fixture.database);
    let dry = handles
        .export
        .execute(true, None)
        .expect("shared full dry-run reproduces every original ID");
    assert_eq!(
        (dry.revisions, dry.git_head.as_str()),
        (count as u64, fixture.history.source_head.as_str())
    );
    assert!(
        text(&fixture.destination, &["for-each-ref"]).is_empty(),
        "dry-run never publishes a ref"
    );
    let export = handles
        .export
        .execute(false, None)
        .expect("verified originals publish atomically at target lease");
    assert_eq!(export.git_head, fixture.history.source_head);
    assert_eq!(
        text(&fixture.destination, &["rev-parse", TARGET]),
        fixture.history.source_head
    );
    assert_eq!(
        text(&fixture.destination, &["rev-list", "--reverse", TARGET]),
        fixture
            .history
            .revisions
            .iter()
            .map(|r| r.original.as_ref().unwrap().oid.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    for object in &fixture.history.objects {
        let kind = match object.kind {
            HistoryKind::Blob => "blob",
            HistoryKind::Tree => "tree",
            _ => unreachable!(),
        };
        assert_eq!(
            git(&fixture.destination, &["cat-file", kind, &object.oid], b""),
            object.bytes
        );
    }
    for revision in &fixture.history.revisions {
        let raw = revision.original.as_ref().unwrap();
        assert_eq!(
            git(&fixture.destination, &["cat-file", "commit", &raw.oid], b""),
            raw.bytes
        );
    }
    assert_eq!(
        fingerprint(&fixture.database),
        before,
        "export is read-only including all provenance/unreachable rows"
    );
}

pub(super) fn archive(fixture: &Fixture, archive: &str) {
    let head = text(&fixture.repo, &["rev-parse", BOARD]);
    let root = text(&fixture.repo, &["rev-parse", &format!("{head}^{{tree}}")]);
    let listing = String::from_utf8(git(&fixture.repo, &["ls-tree", &root], b"")).unwrap();
    let mut listing: String = listing
        .lines()
        .filter(|line| !line.ends_with("\tarchive"))
        .map(|line| format!("{line}\n"))
        .collect();
    listing.push_str(&format!("040000 tree {archive}\tarchive\n"));
    let root = String::from_utf8(git(&fixture.repo, &["mktree"], listing.as_bytes()))
        .unwrap()
        .trim()
        .to_string();
    advance(&fixture.repo, &root, "", "bounds/unsupported fixture\n");
}

#[test]
fn feature_red_linear_merge_is_typed_unsupported_and_source_unchanged() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let head = text(&fixture.repo, &["rev-parse", BOARD]);
    let root = text(&fixture.repo, &["rev-parse", &format!("{head}^{{tree}}")]);
    let ancestor = fixture.history.revisions[0]
        .original
        .as_ref()
        .unwrap()
        .oid
        .clone();
    let merge = commit(
        &fixture.repo,
        &root,
        &[&head, &ancestor],
        "",
        "unsupported merge\n",
    );
    git(&fixture.repo, &["update-ref", BOARD, &merge, &head], b"");
    let before = text(&fixture.repo, &["show-ref"]);
    let result = fixture.b_handles().source.capture();
    assert!(
        matches!(result, Err(HistoryError::Unsupported(_))),
        "linear allowlist must refuse a genuine two-parent commit, not Unavailable/success: {result:?}"
    );
    assert_eq!(text(&fixture.repo, &["show-ref"]), before);
    assert!(!fixture.database.exists());
}
#[test]
fn feature_red_missing_actual_blob_closure_is_typed_malformed() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let missing = "2".repeat(64);
    assert!(fixture.history.objects.iter().all(|o| o.oid != missing));
    let tree = raw_tree(&fixture.repo, &[("100644", b"missing", &missing)]);
    archive(&fixture, &tree);
    let before = text(&fixture.repo, &["show-ref"]);
    let result = fixture.b_handles().source.capture();
    assert!(
        matches!(
            result,
            Err(HistoryError::Malformed(_) | HistoryError::Unsupported(_))
        ),
        "missing non-gitlink object must wholly refuse: {result:?}"
    );
    assert_eq!(text(&fixture.repo, &["show-ref"]), before);
    assert!(!fixture.database.exists());
}
#[test]
fn feature_red_non_gitlink_actual_commit_is_not_a_blob_tree() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let tree = raw_tree(
        &fixture.repo,
        &[("100644", b"commit-not-blob", &fixture.history.source_head)],
    );
    archive(&fixture, &tree);
    let result = fixture.b_handles().source.capture();
    assert!(
        matches!(result, Err(HistoryError::Unsupported(_))),
        "actual commit in non-gitlink closure must be unsupported: {result:?}"
    );
    assert!(!fixture.database.exists());
}
fn raw_name_bound(size: usize) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let blob = hash(&fixture.repo, "blob", b"name bound");
    let tree = raw_tree(&fixture.repo, &[("100644", &vec![b'n'; size], &blob)]);
    archive(&fixture, &tree);
    let result = fixture.b_handles().source.capture();
    if size == 4096 {
        let captured = result.expect("inclusive raw-name boundary");
        assert!(
            captured
                .objects
                .iter()
                .any(|o| o.entries.iter().any(|r| r.name.len() == 4096))
        );
    } else {
        assert!(
            matches!(result, Err(HistoryError::Bounds(_))),
            "raw-name +1 whole refusal: {result:?}"
        );
    }
}
#[test]
fn feature_red_raw_name_4096_inclusive() {
    raw_name_bound(4096);
}
#[test]
fn feature_red_raw_name_4097_typed_bounds() {
    raw_name_bound(4097);
}
fn commit_bound(size: usize) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let parent = text(&fixture.repo, &["rev-parse", BOARD]);
    let root = text(&fixture.repo, &["rev-parse", &format!("{parent}^{{tree}}")]);
    let empty = commit(&fixture.repo, &root, &[&parent], "", "");
    let base = git(&fixture.repo, &["cat-file", "commit", &empty], b"").len();
    let next = advance(&fixture.repo, &root, "", &"m".repeat(size - base));
    assert_eq!(
        git(&fixture.repo, &["cat-file", "commit", &next], b"").len(),
        size
    );
    let result = fixture.b_handles().source.capture();
    if size == 65536 {
        assert_eq!(
            result
                .expect("inclusive commit64KiB")
                .revisions
                .last()
                .unwrap()
                .original
                .as_ref()
                .unwrap()
                .bytes
                .len(),
            size
        );
    } else {
        assert!(
            matches!(result, Err(HistoryError::Bounds(_))),
            "commit +1 typed bounds: {result:?}"
        );
    }
}
#[test]
fn feature_red_commit_65536_inclusive() {
    commit_bound(65536);
}
#[test]
fn feature_red_commit_65537_typed_bounds() {
    commit_bound(65537);
}
#[test]
fn feature_red_actual_tree_depth_beyond_128_is_refused_without_partial_import() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let mut tree = raw_tree(&fixture.repo, &[]);
    for _ in 0..129 {
        tree = raw_tree(&fixture.repo, &[("40000", b"next", &tree)]);
    }
    archive(&fixture, &tree);
    let result = fixture.b_handles().source.capture();
    assert!(
        matches!(result, Err(HistoryError::Bounds(_))),
        "recursive closure beyond depth128 must refuse: {result:?}"
    );
    assert!(!fixture.database.exists());
}

#[test]
fn feature_red_prepared_view_exports_before_commit_and_drop_rolls_back_every_row() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    assert_eq!(store.read(), Err(BoardStoreError::NoBoard));
    let before = fingerprint(&fixture.database);
    let handles = fixture.b_handles();
    let prepared = handles
        .store
        .prepare_import(&fixture.history)
        .expect("one owned prepared IMMEDIATE import");
    assert_eq!(prepared.frontier().unwrap(), BoardHead("1:12".into()));
    assert_eq!(
        store.read(),
        Err(BoardStoreError::NoBoard),
        "uncommitted rows invisible on independent client"
    );
    let export = handles
        .export
        .execute_snapshot(&*prepared, true, None)
        .expect("same complete exporter checks prepared uncommitted view");
    assert_eq!(
        (export.revisions, export.git_head),
        (12, fixture.history.source_head.clone())
    );
    assert_eq!(
        prepared.next_revision(0).unwrap().unwrap().original,
        fixture.history.revisions[0].original
    );
    for expected in &fixture.history.revisions {
        let actual = prepared
            .next_revision(expected.sequence - 1)
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                actual.sequence,
                actual.parent,
                &actual.root,
                &actual.original
            ),
            (
                expected.sequence,
                expected.parent,
                &expected.root,
                &expected.original
            )
        );
    }
    assert!(prepared.next_revision(12).unwrap().is_none());
    for expected in &fixture.history.objects {
        assert_eq!(
            prepared.object(&expected.oid).unwrap(),
            *expected,
            "original raw OID/kind/bytes/ordered tree projection survives uncommitted SQL view"
        );
    }
    drop(prepared);
    assert_eq!(
        fingerprint(&fixture.database),
        before,
        "Drop rolls back objects,entries,revisions,head,raw/provenance/epochs including unreachable rows"
    );
    assert_eq!(store.read(), Err(BoardStoreError::NoBoard));
    let prepared = handles.store.prepare_import(&fixture.history).unwrap();
    let checked = handles
        .export
        .execute_snapshot(&*prepared, true, None)
        .unwrap();
    assert_eq!(checked.git_head, fixture.history.source_head);
    let report = prepared.commit().unwrap();
    assert_eq!(report.head, BoardHead("1:12".into()));
    assert_eq!(store.read().unwrap().head, report.head);
}
#[test]
fn feature_red_nonempty_target_conflict_preserves_all_existing_rows() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let store = native(&fixture);
    let before = fingerprint(&fixture.database);
    let head = store.read().unwrap().head;
    let result = fixture.b_handles().store.prepare_import(&fixture.history);
    assert!(
        matches!(result, Err(HistoryError::Conflict(_))),
        "already bootstrapped target must refuse without overwrite/resume"
    );
    assert_eq!(fingerprint(&fixture.database), before);
    assert_eq!(store.read().unwrap().head, head);
}

#[test]
fn feature_red_native_every_kind_unchanged_and_clear_export_real_deterministic_children() {
    use quecto::infrastructure::test_support::board_artifacts;
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let handles = fixture.b_handles();
    handles
        .import
        .execute()
        .expect("import bridge before native continuation");
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    let lower: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .try_into()
        .unwrap();
    let mut head = store.read().unwrap().head;
    let values = vec![
        BoardArtifact::Task(task("a", "Native task")),
        BoardArtifact::Plan(board_artifacts::plan("a", 1)),
        BoardArtifact::Run(board_artifacts::run("a", 1)),
        BoardArtifact::Change(board_artifacts::change("a", 1)),
        BoardArtifact::Review(board_artifacts::review("a", 1)),
        BoardArtifact::Discussion(board_artifacts::discussion("store-locking")),
    ];
    head = written(store.write(&head, &values, "board: native all supported artifacts"));
    head = written(store.write(&head, &values, "board: unchanged child remains"));
    assert_eq!(store.read().unwrap().head, head);
    store.fixture_put_raw("tasks/broken.md", b"{").unwrap();
    let snapshot = store.read().unwrap();
    let q = snapshot
        .quarantined
        .iter()
        .find(|q| q.path == "tasks/broken.md")
        .unwrap();
    let clear = SetAsideClear {
        path: q.path.clone(),
        object: q.object.clone(),
        repeated: Default::default(),
    };
    head = written(store.clear_set_aside(&snapshot.head, &clear, "board: native clear"));
    let upper: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .try_into()
        .unwrap();
    let first = handles
        .export
        .execute(false, None)
        .expect("native framed IDs translate into real Git objects");
    assert_eq!(first.frontier, head);
    assert_eq!(first.revisions, 16);
    let git_head = text(&fixture.destination, &["rev-parse", TARGET]);
    assert_eq!(git_head, first.git_head);
    assert_ne!(git_head, head.0);
    assert_eq!(
        text(
            &fixture.destination,
            &["rev-parse", &format!("{git_head}~4")]
        ),
        fixture.history.source_head,
        "first native parent bridges H0"
    );
    assert_eq!(
        text(
            &fixture.destination,
            &["log", "-1", "--format=%s", &git_head]
        ),
        "board: native clear"
    );
    let ids = text(&fixture.destination, &["rev-list", "--reverse", TARGET]);
    assert_eq!(ids.lines().count(), 16);
    let native_ids: Vec<_> = ids.lines().skip(12).collect();
    assert_eq!(native_ids.len(), 4);
    assert_ne!(native_ids[0], native_ids[1]);
    assert_eq!(
        text(
            &fixture.destination,
            &["rev-parse", &format!("{}^{{tree}}", native_ids[0])]
        ),
        text(
            &fixture.destination,
            &["rev-parse", &format!("{}^{{tree}}", native_ids[1])]
        ),
        "unchanged writes export the same real tree but distinct parented commits"
    );
    let persisted = handles.store.snapshot().unwrap();
    for (index, id) in native_ids.into_iter().enumerate() {
        let revision = persisted.next_revision(12 + index as u64).unwrap().unwrap();
        assert_eq!(
            (revision.sequence, revision.parent),
            (13 + index as u64, Some(12 + index as u64))
        );
        assert!(revision.original.is_none());
        assert_eq!(revision.writer, writer());
        assert!(
            (lower..=upper).contains(&revision.at),
            "native UTC seconds captured with the successful mutation, not arbitrary fixed metadata"
        );
        match index {
            0 => assert_eq!(revision.message, "board: native all supported artifacts"),
            1 => assert_eq!(revision.message, "board: unchanged child remains"),
            3 => assert_eq!(revision.message, "board: native clear"),
            2 => assert!(
                !revision.message.is_empty(),
                "fixture revision has explicit durable message"
            ),
            _ => unreachable!(),
        };
        let raw = git(&fixture.destination, &["cat-file", "commit", id], b"");
        let expected_parent = if index == 0 {
            fixture.history.source_head.as_str()
        } else {
            ids.lines().nth(11 + index).unwrap()
        };
        let raw_text = String::from_utf8(raw.clone()).unwrap();
        assert_eq!(
            raw_text
                .lines()
                .filter_map(|line| line.strip_prefix("parent "))
                .collect::<Vec<_>>(),
            vec![expected_parent],
            "each native child uses the previous actual Git commit, not SQL head/root"
        );
        let expected_author = format!(
            "author {} <{}> {} +0000\ncommitter {} <{}> {} +0000\n",
            revision.writer.name,
            revision.writer.email,
            revision.at,
            revision.writer.name,
            revision.writer.email,
            revision.at
        );
        assert!(
            raw.windows(expected_author.len())
                .any(|bytes| bytes == expected_author.as_bytes()),
            "export uses persisted writer and UTCtime, not current clock/config"
        );
        assert!(raw.ends_with(format!("\n\n{}\n", revision.message).as_bytes()));
        let root = text(
            &fixture.destination,
            &["rev-parse", &format!("{id}^{{tree}}")],
        );
        assert_eq!(
            text(&fixture.destination, &["cat-file", "-t", &root]),
            "tree"
        );
        assert_eq!(
            persisted.object(&revision.root).unwrap().kind,
            HistoryKind::Tree
        );
        assert_ne!(
            root, revision.root,
            "native framed root is recursively translated, not hashed as if already a Git tree"
        );
    }
    for (path, artifact) in [
        "tasks/a.md",
        "plans/a.1.md",
        "runs/a.1.md",
        "changes/a.1.md",
        "reviews/a.1.md",
        "discussions/store-locking.md",
    ]
    .into_iter()
    .zip(&values)
    {
        let expected =
            quecto::infrastructure::persistence::project_board::artifacts::file_of(artifact)
                .unwrap();
        assert_eq!(
            git(
                &fixture.destination,
                &["show", &format!("{git_head}:{path}")],
                b""
            ),
            expected,
            "every native kind byte-exact in real Git closure"
        );
    }
    let before = fingerprint(&fixture.database);
    let repeated = handles.export.execute(true, Some(&git_head)).unwrap();
    assert_eq!(
        repeated.git_head, git_head,
        "no current wallclock/config/signature changes deterministic export"
    );
    assert_eq!(fingerprint(&fixture.database), before);
    assert_eq!(
        text(
            &fixture.destination,
            &["rev-parse", &format!("{git_head}:archive")]
        ),
        text(
            &fixture.repo,
            &[
                "rev-parse",
                &format!("{}:archive", fixture.history.source_head)
            ]
        )
    );
}

#[test]
fn feature_red_lease_conflict_does_not_publish_a_verified_partial_history() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let handles = fixture.b_handles();
    handles
        .import
        .execute()
        .expect("import before publication race");
    let before = fingerprint(&fixture.database);
    let raw = hash(&fixture.destination, "tree", b"");
    let held = commit(&fixture.destination, &raw, &[], "", "destination winner\n");
    git(&fixture.destination, &["update-ref", TARGET, &held], b"");
    let result = handles.export.execute(false, None);
    assert!(
        matches!(result, Err(HistoryError::Conflict(_))),
        "captured absent-head lease must refuse winner, not force update: {result:?}"
    );
    assert_eq!(text(&fixture.destination, &["rev-parse", TARGET]), held);
    assert_eq!(fingerprint(&fixture.database), before);
}

#[test]
fn feature_red_new_offline_cli_import_and_export_additive_to_operator_root() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let home = fixture.directory.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let mut command = Command::new("timeout");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap());
    command
        .args([
            "-s",
            "KILL",
            "60",
            env!("CARGO_BIN_EXE_quecto"),
            "board-server",
            "import",
            "--repo",
            fixture.repo.to_str().unwrap(),
            "--remote",
            "origin",
            "--board-id",
            "0123456789abcdef",
        ])
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("QUECTO_BASE_DIR", &home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "NEW B3 offline import feature absent (narrow dispatch RED only): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["revisions"], 12);
    assert_eq!(report["source_head"], fixture.history.source_head);
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
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("QUECTO_BASE_DIR", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "offline full rollback dry-run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["revisions"], 12);
    assert_eq!(report["git_head"], fixture.history.source_head);
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
    let options = quecto::interface::cli::board_server::serve::parse(&[
        "serve".into(),
        "--socket".into(),
        home.join("not-started.sock").display().to_string(),
        "--repo".into(),
        fixture.repo.display().to_string(),
    ])
    .unwrap();
    assert_eq!(
        options.repo, fixture.repo,
        "existing operator serve remains parseable without starting server"
    );
}

#[test]
fn feature_green_source_refuses_missing_required_commit_headers_and_blob_root() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let root = &fixture.history.revisions.last().unwrap().root;
    let blob = hash(&fixture.repo, "blob", b"not a commit root tree");
    for (case, body) in [
        (
            "author",
            format!(
                "tree {root}\ncommitter History <history@example.com> 1700000000 +0000\n\nno author\n"
            ),
        ),
        (
            "committer",
            format!(
                "tree {root}\nauthor History <history@example.com> 1700000000 +0000\n\nno committer\n"
            ),
        ),
        (
            "root",
            format!(
                "tree {blob}\nauthor History <history@example.com> 1700000000 +0000\ncommitter History <history@example.com> 1700000000 +0000\n\nblob root\n"
            ),
        ),
    ] {
        let bad = hash(&fixture.repo, "commit", body.as_bytes());
        // A literal malformed commit is the fixture, not a live/ref publication.
        // This bare repository belongs solely to the enclosing TempDir.
        std::fs::write(fixture.repo.join(BOARD), format!("{bad}\n")).unwrap();
        let result = fixture.b_handles().source.capture();
        assert!(
            matches!(result, Err(HistoryError::Malformed(_))),
            "missing/invalid required {case} must wholly refuse: {result:?}"
        );
    }
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
    assert!(!fixture.database.exists());
}
