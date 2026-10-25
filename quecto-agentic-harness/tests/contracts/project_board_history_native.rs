//! Additional A native-only requirements; no original imported continuation is replaced.
use super::project_board_history_fixture::*;
use super::project_board_store::task;
use quecto::application::project_board::ports::{history::*, *};
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;

fn written(result: Result<BoardWrite, BoardStoreError>) -> BoardHead {
    match result.unwrap() {
        BoardWrite::Written(head) => head,
        other => panic!("expected native mutation: {other:?}"),
    }
}
fn native(fixture: &Fixture) -> SqliteProjectBoardStore {
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    store.bootstrap().unwrap();
    store
}
fn seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .try_into()
        .unwrap()
}
fn native_export(algorithm: HistoryAlgorithm) {
    use quecto::infrastructure::test_support::board_artifacts;
    let fixture = Fixture::new(algorithm, 12);
    fixture.verify();
    let lower = seconds();
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
    let head = written(store.write(&head, &values, "board: native all supported artifacts"));
    let head = written(store.write(&head, &values, "board: unchanged child remains"));
    store.fixture_put_raw("tasks/broken.md", b"{").unwrap();
    let snapshot = store.read().unwrap();
    assert_ne!(snapshot.head, head);
    let q = snapshot
        .quarantined
        .iter()
        .find(|q| q.path == "tasks/broken.md")
        .unwrap();
    let head = written(store.clear_set_aside(
        &snapshot.head,
        &SetAsideClear {
            path: q.path.clone(),
            object: q.object.clone(),
            repeated: Default::default(),
        },
        "board: native clear",
    ));
    assert!(store.read().unwrap().quarantined.is_empty());
    let upper = seconds();
    let before = fingerprint(&fixture.database);
    let handles = fixture.handles();
    let report = handles
        .export
        .execute(false, None)
        .expect("complete native export in selected destination format");
    assert_eq!((report.revisions, report.frontier), (5, head));
    assert_eq!(
        report.git_head.len(),
        match algorithm {
            HistoryAlgorithm::Sha1 => 40,
            HistoryAlgorithm::Sha256 => 64,
        }
    );
    let ids = text(&fixture.destination, &["rev-list", "--reverse", TARGET]);
    let ids: Vec<_> = ids.lines().collect();
    assert_eq!(ids.len(), 5);
    assert_eq!(ids[4], report.git_head);
    assert_eq!(
        git(
            &fixture.destination,
            &["show", &format!("{}:tasks/broken.md", ids[3])],
            b""
        ),
        b"{",
        "the pre-clear exported revision retains the exact set-aside payload"
    );
    let cleared_paths = git(
        &fixture.destination,
        &["ls-tree", "-r", "-z", "--name-only", ids[4]],
        b"",
    );
    assert_eq!(
        cleared_paths
            .split(|byte| *byte == 0)
            .filter(|path| *path == b"tasks/broken.md")
            .count(),
        0,
        "the actual clear child must omit broken.md, not reuse its pre-clear tree"
    );
    assert_ne!(ids[1], ids[2]);
    assert_eq!(
        text(
            &fixture.destination,
            &["rev-parse", &format!("{}^{{tree}}", ids[1])]
        ),
        text(
            &fixture.destination,
            &["rev-parse", &format!("{}^{{tree}}", ids[2])]
        )
    );
    let persisted = handles.store.snapshot().unwrap();
    assert_eq!(
        persisted.algorithm().unwrap(),
        None,
        "positive native format neutrality permits both destination formats"
    );
    for (index, id) in ids.iter().enumerate() {
        let revision = persisted.next_revision(index as u64).unwrap().unwrap();
        assert_eq!(
            (revision.sequence, revision.parent),
            (index as u64 + 1, index.checked_sub(1).map(|n| n as u64 + 1))
        );
        assert!(revision.original.is_none());
        assert_eq!(revision.writer, writer());
        assert!((lower..=upper).contains(&revision.at));
        match index {
            0 => assert_eq!(revision.message, "board: bootstrap"),
            1 => assert_eq!(revision.message, "board: native all supported artifacts"),
            2 => assert_eq!(revision.message, "board: unchanged child remains"),
            3 => assert!(!revision.message.is_empty()),
            4 => assert_eq!(revision.message, "board: native clear"),
            _ => unreachable!(),
        }
        let raw = git(&fixture.destination, &["cat-file", "commit", id], b"");
        let raw_text = String::from_utf8(raw.clone()).unwrap();
        let parents: Vec<_> = raw_text
            .lines()
            .filter_map(|l| l.strip_prefix("parent "))
            .collect();
        assert_eq!(
            parents,
            index
                .checked_sub(1)
                .map(|n| ids[n])
                .into_iter()
                .collect::<Vec<_>>(),
            "actual preceding Git parent, never framed root or epoch head"
        );
        let metadata = format!(
            "author {} <{}> {} +0000\ncommitter {} <{}> {} +0000\n",
            revision.writer.name,
            revision.writer.email,
            revision.at,
            revision.writer.name,
            revision.writer.email,
            revision.at
        );
        assert!(
            raw.windows(metadata.len())
                .any(|b| b == metadata.as_bytes())
        );
        assert!(raw.ends_with(format!("\n\n{}\n", revision.message).as_bytes()));
        let root = text(
            &fixture.destination,
            &["rev-parse", &format!("{id}^{{tree}}")],
        );
        assert_ne!(root, revision.root);
        assert_eq!(
            persisted.object(&revision.root).unwrap().kind,
            HistoryKind::Tree
        );
    }
    assert!(persisted.next_revision(5).unwrap().is_none());
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
        assert_eq!(
            git(
                &fixture.destination,
                &["show", &format!("{}:{path}", report.git_head)],
                b""
            ),
            quecto::infrastructure::persistence::project_board::artifacts::file_of(artifact)
                .unwrap()
        );
    }
    let dry_objects = text(
        &fixture.destination,
        &["cat-file", "--batch-all-objects", "--batch-check"],
    );
    let dry_refs = text(&fixture.destination, &["for-each-ref"]);
    assert_eq!(
        handles
            .export
            .execute(true, Some(&report.git_head))
            .unwrap()
            .git_head,
        report.git_head
    );
    assert_eq!(
        text(
            &fixture.destination,
            &["cat-file", "--batch-all-objects", "--batch-check"]
        ),
        dry_objects
    );
    assert_eq!(text(&fixture.destination, &["for-each-ref"]), dry_refs);
    assert_eq!(fingerprint(&fixture.database), before);
}
#[test]
fn native_six_kinds_unchanged_clear_utc_writer_messages_actual_parents_sha1() {
    native_export(HistoryAlgorithm::Sha1);
}
#[test]
fn native_six_kinds_unchanged_clear_utc_writer_messages_actual_parents_sha256() {
    native_export(HistoryAlgorithm::Sha256);
}

#[test]
fn native_schema4_has_exact_nine_tables_seven_indexes_and_positive_metadata() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let _store = native(&fixture);
    let c = rusqlite::Connection::open(&fixture.database).unwrap();
    let version: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        version, 4,
        "native durable history requires schema4 (B3's history, B7's op answers)"
    );
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        tables,
        [
            "epochs",
            "git_objects",
            "history_meta",
            "objects",
            "op_results",
            "revision_principals",
            "revisions",
            "state",
            "tree_entries"
        ]
    );
    let indexes: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='index'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 7);
    let meta: (Option<String>, String, Option<String>, Option<String>, Option<String>, i64) = c.query_row("SELECT board_id,algorithm,source_remote,source_ref,source_head,imported_count FROM history_meta", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(meta, (None, "sha256".into(), None, None, None, 0));
    let git_fields: i64 = c
        .query_row(
            "SELECT count(*) FROM revisions WHERE git_commit IS NOT NULL OR git_meta IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(git_fields, 0);
    assert_eq!(
        fixture
            .handles()
            .store
            .snapshot()
            .unwrap()
            .algorithm()
            .unwrap(),
        None
    );
}
fn corrupt_native(sql: &str) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let _store = native(&fixture);
    let handles = fixture.handles();
    let valid = handles
        .store
        .snapshot()
        .expect("real native snapshot before corrupt fixture setup");
    assert_eq!(valid.algorithm().unwrap(), None);
    drop(valid);
    let c = rusqlite::Connection::open(&fixture.database).unwrap();
    c.execute_batch(sql).unwrap();
    c.close().unwrap();
    let before = fingerprint(&fixture.database);
    let result = handles.store.snapshot();
    match result {
        Err(
            HistoryError::Malformed(_) | HistoryError::Unsupported(_) | HistoryError::Conflict(_),
        ) => {}
        Ok(view) => panic!(
            "corrupt native metadata must not become a None fallback: {:?}",
            view.algorithm()
        ),
        Err(error) => {
            panic!("corruption must have a precise refusal, not missing feature: {error:?}")
        }
    }
    assert_eq!(fingerprint(&fixture.database), before);
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
}
#[test]
fn native_metadata_unknown_algorithm_refuses_without_none_fallback() {
    corrupt_native("UPDATE history_meta SET algorithm='unknown'");
}
#[test]
fn native_metadata_source_fields_refuse_without_none_fallback() {
    corrupt_native(
        "UPDATE history_meta SET source_remote='origin',source_ref='refs/heads/quecto/board',source_head='ffffffffffffffffffffffffffffffffffffffff'",
    );
}
#[test]
fn native_metadata_count_refuses_without_none_fallback() {
    corrupt_native("UPDATE history_meta SET imported_count=1");
}
#[test]
fn native_metadata_git_revision_fields_refuse_without_none_fallback() {
    corrupt_native(
        "UPDATE revisions SET git_commit='ffffffffffffffffffffffffffffffffffffffff',git_meta=X'00'",
    );
}
#[test]
fn native_metadata_original_members_refuse_without_none_fallback() {
    corrupt_native(
        "INSERT INTO git_objects VALUES ('ffffffffffffffffffffffffffffffffffffffff',NULL)",
    );
}
#[test]
fn native_metadata_wrong_board_binding_refuses_without_mutation() {
    corrupt_native("UPDATE history_meta SET board_id='fedcba9876543210'");
}
#[test]
fn native_populated_import_provenance_is_explicitly_unsupported_until_b() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let _store = native(&fixture);
    let handles = fixture.handles();
    let valid = handles
        .store
        .snapshot()
        .expect("native admission before populated import provenance fixture");
    assert_eq!(valid.algorithm().unwrap(), None);
    drop(valid);
    // A recognizes the populated source manifest but does not implement import.
    let c = rusqlite::Connection::open(&fixture.database).unwrap();
    c.execute("UPDATE history_meta SET board_id='0123456789abcdef',algorithm='sha1',source_remote='origin',source_ref=?1,source_head=?2,imported_count=12", rusqlite::params![BOARD,fixture.history.source_head]).unwrap();
    c.close().unwrap();
    let before = fingerprint(&fixture.database);
    assert!(matches!(
        handles.store.snapshot(),
        Err(HistoryError::Unsupported(_))
    ));
    assert_eq!(fingerprint(&fixture.database), before);
}
#[test]
fn native_readonly_snapshot_missing_replaced_and_old_view_pinning() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let store = native(&fixture);
    let handles = fixture.handles();
    let pinned = handles
        .store
        .snapshot()
        .expect("owned readonly pinned native view");
    let frontier = pinned.frontier().unwrap();
    let first = pinned.next_revision(0).unwrap().unwrap();
    let before = fingerprint(&fixture.database);
    assert_eq!(pinned.algorithm().unwrap(), None);
    assert_eq!(fingerprint(&fixture.database), before);
    let next = written(store.write_tasks(
        &frontier,
        &[task("a", "After pinned native snapshot")],
        "board: concurrent pinned write",
    ));
    assert_ne!(next, frontier);
    assert_eq!(pinned.frontier().unwrap(), frontier);
    assert_eq!(pinned.next_revision(0).unwrap().unwrap(), first);
    assert!(pinned.next_revision(1).unwrap().is_none());
    // Release all connections before moving the exact scratch DB path. This
    // tests missing/replaced admission without reusing an active WAL pathname.
    drop(pinned);
    drop(store);
    let saved = fixture.database.with_extension("saved");
    std::fs::rename(&fixture.database, &saved).unwrap();
    assert!(matches!(
        handles.store.snapshot(),
        Err(HistoryError::Unavailable(_))
    ));
    assert!(
        !fixture.database.exists(),
        "readonly cannot initialize missing path"
    );
    let c = rusqlite::Connection::open(&fixture.database).unwrap();
    c.execute_batch("CREATE TABLE foreign_store(bytes BLOB); PRAGMA user_version=900;")
        .unwrap();
    c.close().unwrap();
    let bytes = std::fs::read(&fixture.database).unwrap();
    assert!(matches!(
        handles.store.snapshot(),
        Err(HistoryError::Unsupported(_) | HistoryError::Conflict(_))
    ));
    assert_eq!(std::fs::read(&fixture.database).unwrap(), bytes);
}

#[test]
fn native_lease_conflict_preserves_winner_and_all_database_rows() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let _store = native(&fixture);
    let before = fingerprint(&fixture.database);
    let handles = fixture.handles();
    let tree = hash(&fixture.destination, "tree", b"");
    let winner = commit(&fixture.destination, &tree, &[], "", "destination winner\n");
    git(&fixture.destination, &["update-ref", TARGET, &winner], b"");
    let result = handles.export.execute(false, None);
    assert!(
        matches!(result, Err(HistoryError::Conflict(_))),
        "captured absent target lease must refuse a subsequent winner: {result:?}"
    );
    assert_eq!(text(&fixture.destination, &["rev-parse", TARGET]), winner);
    assert_eq!(fingerprint(&fixture.database), before);
}
#[test]
fn native_corrupt_payload_refuses_export_without_ref_or_database_mutation() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let store = native(&fixture);
    let head = store.read().unwrap().head;
    written(store.write_tasks(
        &head,
        &[task("a", "Corrupt payload")],
        "board: valid before corruption",
    ));
    let handles = fixture.handles();
    let valid = handles
        .store
        .snapshot()
        .expect("real native view before corruption setup");
    assert_eq!(valid.algorithm().unwrap(), None);
    drop(valid);
    let c = rusqlite::Connection::open(&fixture.database).unwrap();
    let changed = c
        .execute(
            "UPDATE objects SET bytes=zeroblob(length(bytes)) WHERE kind='blob'",
            [],
        )
        .unwrap();
    assert!(changed > 0);
    c.close().unwrap();
    let before = fingerprint(&fixture.database);
    assert!(matches!(
        handles.export.execute(false, None),
        Err(HistoryError::Malformed(_))
    ));
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
    assert_eq!(fingerprint(&fixture.database), before);
}
fn cli(home: &std::path::Path, args: &[String]) -> std::process::Output {
    std::process::Command::new("timeout")
        .args(["-s", "KILL", "60", env!("CARGO_BIN_EXE_quecto")])
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("QUECTO_BASE_DIR", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}
#[test]
fn offline_export_help_advertises_export_only_and_preserves_operator_verbs() {
    let home = tempfile::tempdir().unwrap();
    let output = cli(home.path(), &["--help".into()]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(
        help.contains("board-server export"),
        "export must be registered in actual root help"
    );
    assert!(help.contains("board-server serve"));
    assert!(help.contains("board-server token"));
    assert!(
        help.contains("board-server import --repo <source> --board-id <16-hex> [--remote <name>]"),
        "B complete offline import usage must be registered in actual root help"
    );
}
#[test]
fn offline_export_allowlist_format3_board_id_path_confinement_and_pure_dryrun() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    let home = fixture.directory.path().join("offline-home");
    let boards = home.join("board-server/0123456789abcdef");
    std::fs::create_dir_all(&boards).unwrap();
    let database = boards.join("board.sqlite3");
    let store = SqliteProjectBoardStore::open(&database, writer()).unwrap();
    store.bootstrap().unwrap();
    let before = fingerprint(&database);
    let inventory = text(
        &fixture.destination,
        &["cat-file", "--batch-all-objects", "--batch-check"],
    );
    let args: Vec<String> = [
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
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let output = cli(&home, &args);
    assert!(
        output.status.success(),
        "offline export must dispatch before server/token validation: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["revisions"], 1);
    assert_eq!(report["frontier"], "1:1");
    assert_eq!(report["git_head"].as_str().unwrap().len(), 40);
    for bad_id in [
        "../escape",
        "/absolute",
        "0123456789abcdeg",
        "0123",
        "0123456789abcdef00",
    ] {
        let mut bad = args.clone();
        bad[5] = bad_id.into();
        let rejected = cli(&home, &bad);
        assert_eq!(rejected.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("board-id"));
    }
    for format in ["2", "4", "03"] {
        let mut bad = args.clone();
        bad[9] = format.into();
        let rejected = cli(&home, &bad);
        assert_eq!(rejected.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("format"));
    }
    for extra in [
        vec!["--dry-run"],
        vec!["--unknown", "value"],
        vec!["--board-id", "0123456789abcdef"],
        vec!["--repo"],
    ] {
        let mut bad = args.clone();
        bad.extend(extra.into_iter().map(str::to_owned));
        assert_eq!(cli(&home, &bad).status.code(), Some(1));
    }
    let mut bad_target = args.clone();
    bad_target[7] = "../escape".into();
    assert_eq!(cli(&home, &bad_target).status.code(), Some(1));
    let mut import = args.clone();
    import[1] = "import".into();
    assert_eq!(cli(&home, &import).status.code(), Some(1));
    assert_eq!(
        text(
            &fixture.destination,
            &["cat-file", "--batch-all-objects", "--batch-check"]
        ),
        inventory,
        "dry-run writes no unreachable objects either"
    );
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
    assert_eq!(fingerprint(&database), before);
    let files: Vec<_> = std::fs::read_dir(&boards)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        files
            .iter()
            .all(|n| n.to_string_lossy().starts_with("board.sqlite3"))
    );
    assert!(!home.join("escape").exists());
}
