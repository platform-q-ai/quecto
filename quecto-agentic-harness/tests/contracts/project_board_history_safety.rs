//! Scoped A sink safety; original import safety deferred intact.
use super::project_board_history_fixture::*;
use quecto::application::project_board::ports::history::*;
use quecto::application::project_board::ports::{BoardHead, BoardStoreError, ProjectBoardStore};
use quecto::infrastructure::persistence::sqlite::project_board::SqliteProjectBoardStore;
use std::path::Path;
fn assert_projection(entries: &[(&str, &[u8], &str)]) {
    let allocation: usize = entries
        .iter()
        .map(|(_, name, oid)| 128 + 6 + oid.len() + 4 * name.len())
        .sum();
    assert!(
        allocation <= 4 * 655360,
        "positive boundary fixture must satisfy independent normalized listing allocation guard: {allocation}"
    );
    let logical: usize = entries
        .iter()
        .map(|(mode, name, oid)| {
            mode.trim_start_matches('0').len() + 1 + name.len() + 1 + oid.len() / 2
        })
        .sum();
    assert!(
        logical <= 655360,
        "positive fixture independently fits logical tree admission"
    );
}

fn checked_tree(repo: &Path, entries: &[(&str, &[u8], &str)]) -> String {
    assert_projection(entries);
    raw_tree(repo, entries)
}

#[test]
fn feature_red_sink_native_tree_uses_git_directory_sort_and_binary_oid_width() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let blob = hash(&fixture.destination, "blob", b"native bytes\0");
    let directory = checked_tree(&fixture.destination, &[("100644", b"child", &blob)]);
    let entries = vec![
        HistoryEntry {
            mode: "040000".into(),
            name: b"a".to_vec(),
            oid: directory.clone(),
        },
        HistoryEntry {
            mode: "100644".into(),
            name: b"a.c".to_vec(),
            oid: blob.clone(),
        },
    ];
    let listing = format!("040000 tree {directory}\ta\n100644 blob {blob}\ta.c\n");
    let expected = String::from_utf8(git(&fixture.destination, &["mktree"], listing.as_bytes()))
        .unwrap()
        .trim()
        .to_string();
    let actual = fixture
        .handles()
        .sink
        .tree(&entries, true)
        .expect("pure real Git tree codec");
    assert_eq!(actual, expected);
    let bytes = git(&fixture.destination, &["cat-file", "tree", &expected], b"");
    assert!(
        bytes.starts_with(b"100644 a.c\0"),
        "directory-aware order differs from lexical a-before-a.c"
    );
    assert_eq!(bytes.len(), 6 + 1 + 3 + 1 + 20 + 5 + 1 + 1 + 1 + 20);
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
}

#[test]
fn feature_red_sink_wrong_original_oid_is_malformed_not_published() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let result = fixture.handles().sink.raw(
        HistoryKind::Blob,
        b"correct body",
        Some(&"0".repeat(40)),
        false,
    );
    assert!(
        matches!(result, Err(HistoryError::Malformed(_))),
        "expected raw OID must be verified against full kind+bytes before any ref publication: {result:?}"
    );
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
}

use super::project_board_history::archive;
use std::{
    collections::BTreeSet,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn capture_bound(fixture: &Fixture, allowed: bool, check: impl FnOnce(LinearHistory)) {
    let refs = text(&fixture.repo, &["show-ref"]);
    let result = fixture.b_handles().source.capture();
    if allowed {
        check(result.expect("inclusive approved finite import envelope"));
    } else {
        assert!(
            matches!(result, Err(HistoryError::Bounds(_))),
            "whole-import typed bounds refusal, not unavailable/truncation/success: {result:?}"
        );
    }
    assert_eq!(text(&fixture.repo, &["show-ref"]), refs);
    assert!(!fixture.database.exists());
}
fn commit_count(count: usize) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let root = fixture.history.revisions.last().unwrap().root.clone();
    let mut previous = fixture.history.source_head.clone();
    for index in 12..count {
        previous = commit(
            &fixture.repo,
            &root,
            &[&previous],
            "",
            &format!("bounded commit {index}\n"),
        );
    }
    git(
        &fixture.repo,
        &["update-ref", BOARD, &previous, &fixture.history.source_head],
        b"",
    );
    assert_eq!(
        text(&fixture.repo, &["rev-list", "--count", BOARD]),
        count.to_string()
    );
    capture_bound(&fixture, count == 4096, |history| {
        assert_eq!(history.revisions.len(), count);
        assert_eq!(history.source_head, previous);
    });
}
#[test]
fn feature_red_commit_count_4096_inclusive() {
    commit_count(4096);
}
#[test]
fn feature_red_commit_count_4097_whole_refusal() {
    commit_count(4097);
}

fn object_count(count: usize) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let existing: BTreeSet<_> = fixture
        .history
        .objects
        .iter()
        .map(|o| o.oid.clone())
        .collect();
    // Four child trees, their archive parent, and a new board root. Commit
    // bodies are separately counted, not part of the blob/tree object cap.
    let blobs = count - existing.len() - 6;
    let files = fixture.directory.path().join("objects");
    std::fs::create_dir(&files).unwrap();
    let mut paths = String::new();
    for index in 0..blobs {
        let path = files.join(index.to_string());
        std::fs::write(&path, format!("unique bounded blob {index}\n")).unwrap();
        paths.push_str(path.to_str().unwrap());
        paths.push('\n');
    }
    let output = git(
        &fixture.repo,
        &["hash-object", "-w", "--stdin-paths"],
        paths.as_bytes(),
    );
    let ids: Vec<_> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(ids.len(), blobs);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), blobs);
    assert!(ids.iter().all(|id| !existing.contains(id)));
    let mut children = vec![];
    for (group, chunk) in ids.chunks(blobs.div_ceil(4)).enumerate() {
        let rows: Vec<_> = chunk
            .iter()
            .enumerate()
            .map(|(n, id)| HistoryEntry {
                mode: "100644".into(),
                name: format!("{group}-{n}").into_bytes(),
                oid: id.clone(),
            })
            .collect();
        let raw: Vec<_> = rows
            .iter()
            .map(|r| (r.mode.as_str(), r.name.as_slice(), r.oid.as_str()))
            .collect();
        children.push(checked_tree(&fixture.repo, &raw));
    }
    assert_eq!(children.len(), 4);
    let names: Vec<_> = (0..4).map(|i| format!("objects-{i}")).collect();
    let entries: Vec<_> = children
        .iter()
        .zip(&names)
        .map(|(id, name)| ("40000", name.as_bytes(), id.as_str()))
        .collect();
    let tree = checked_tree(&fixture.repo, &entries);
    archive(&fixture, &tree);
    let root = text(&fixture.repo, &["rev-parse", &format!("{BOARD}^{{tree}}")]);
    let added: BTreeSet<_> = children.iter().cloned().chain([tree, root]).collect();
    assert_eq!(added.len(), 6);
    assert!(added.iter().all(|id| !existing.contains(id)));
    capture_bound(&fixture, count == 32768, |history| {
        assert_eq!(
            history.objects.len(),
            count,
            "every unique actual blob/tree counted once"
        );
    });
}
#[test]
fn feature_red_object_count_32768_inclusive() {
    object_count(32768);
}
#[test]
fn feature_red_object_count_32769_whole_refusal() {
    object_count(32769);
}

fn total_entries(count: usize) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let previous: usize = fixture
        .history
        .objects
        .iter()
        .map(|o| o.entries.len())
        .sum();
    let root_rows = fixture
        .history
        .objects
        .iter()
        .find(|o| o.oid == fixture.history.revisions.last().unwrap().root)
        .unwrap()
        .entries
        .len();
    let additional = count - previous - root_rows - 8;
    let blob = hash(&fixture.repo, "blob", b"entry-bound-shared-blob");
    let mut children = vec![];
    let mut accounted = 0;
    for group in 0..8 {
        let rows = additional / 8 + usize::from(group < additional % 8);
        accounted += rows;
        let names: Vec<_> = (0..rows)
            .map(|n| {
                if n == 0 {
                    format!("group{group}").into_bytes()
                } else {
                    b"x".to_vec()
                }
            })
            .collect();
        let entries: Vec<_> = names
            .iter()
            .map(|name| ("100644", name.as_slice(), blob.as_str()))
            .collect();
        let tree = checked_tree(&fixture.repo, &entries);
        assert!(
            text(&fixture.repo, &["cat-file", "-s", &tree])
                .parse::<usize>()
                .unwrap()
                <= 655360
        );
        children.push(tree);
    }
    assert_eq!(accounted, additional);
    assert_eq!(children.iter().collect::<BTreeSet<_>>().len(), 8);
    let names: Vec<_> = (0..8).map(|i| format!("group{i}")).collect();
    let entries: Vec<_> = children
        .iter()
        .zip(&names)
        .map(|(id, name)| ("40000", name.as_bytes(), id.as_str()))
        .collect();
    let tree = checked_tree(&fixture.repo, &entries);
    archive(&fixture, &tree);
    capture_bound(&fixture, count == 65536, |history| {
        assert_eq!(
            history
                .objects
                .iter()
                .map(|o| o.entries.len())
                .sum::<usize>(),
            count
        );
    });
}
#[test]
fn feature_red_projected_entries_65536_inclusive() {
    total_entries(65536);
}
#[test]
fn feature_red_projected_entries_65537_whole_refusal() {
    total_entries(65537);
}

#[test]
fn feature_red_checked_metadata_32m_refuses_while_raw_and_entry_counts_fit() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let blob = hash(&fixture.repo, "blob", b"bounded metadata shared blob");
    let mut children = vec![];
    let mut total_rows = 0;
    let mut raw_bytes = 0;
    for group in 0..15u8 {
        let mut names = vec![vec![b'n'; 256]; 2000];
        names[0][0] = b'A' + group;
        let entries: Vec<_> = names
            .iter()
            .map(|name| ("100644", name.as_slice(), blob.as_str()))
            .collect();
        let tree = checked_tree(&fixture.repo, &entries);
        let size = text(&fixture.repo, &["cat-file", "-s", &tree])
            .parse::<usize>()
            .unwrap();
        assert!(size <= 655360);
        raw_bytes += size;
        total_rows += entries.len();
        children.push(tree);
    }
    assert!(total_rows < 65536);
    assert!(raw_bytes < 128 * 1024 * 1024);
    assert!(
        total_rows * (128 + 6 + 40 + 4 * 256) > 32 * 1024 * 1024,
        "projected rows alone exceed32MiB, even before commit/object/index metadata"
    );
    let names: Vec<_> = (0..15).map(|i| format!("metadata{i}")).collect();
    let entries: Vec<_> = children
        .iter()
        .zip(&names)
        .map(|(id, name)| ("40000", name.as_bytes(), id.as_str()))
        .collect();
    let tree = checked_tree(&fixture.repo, &entries);
    archive(&fixture, &tree);
    capture_bound(&fixture, false, |_| unreachable!());
}

fn tree_bytes(size: usize) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let blob = hash(&fixture.repo, "blob", b"tree bound");
    let mode = format!("{}100644", "0".repeat(26));
    let fixed = mode.len() + 1 + 1 + 20;
    let mut names = vec![];
    let mut remaining = size;
    while remaining > fixed + 64 + fixed {
        names.push(vec![b'x'; 64]);
        remaining -= fixed + 64;
    }
    assert!(remaining >= fixed);
    names.push(vec![b'y'; remaining - fixed]);
    assert!(names.iter().all(|n| n.len() <= 4096));
    let entries: Vec<_> = names
        .iter()
        .map(|name| (mode.as_str(), name.as_slice(), blob.as_str()))
        .collect();
    assert!(entries.len() < 65536);
    assert_projection(&entries);
    let logical: usize = entries
        .iter()
        .map(|(_, name, oid)| 6 + 1 + name.len() + 1 + oid.len() / 2)
        .sum();
    assert!(
        logical < size,
        "raw padded-mode length differs from normalized logical listing"
    );
    let tree = checked_tree(&fixture.repo, &entries);
    assert_eq!(
        text(&fixture.repo, &["cat-file", "-s", &tree]),
        size.to_string()
    );
    assert!(
        git(&fixture.repo, &["ls-tree", "-z", &tree], b"").starts_with(b"100644 blob "),
        "Git normalizes padded raw modes without expanding projected mode"
    );
    archive(&fixture, &tree);
    capture_bound(&fixture, size == 655360, |history| {
        assert_eq!(
            history
                .objects
                .iter()
                .find(|o| o.oid == tree)
                .unwrap()
                .bytes
                .len(),
            size
        );
    });
}
#[test]
fn feature_red_raw_tree_655360_inclusive() {
    tree_bytes(655360);
}
#[test]
fn feature_red_raw_tree_655361_whole_refusal() {
    tree_bytes(655361);
}

fn blob_bytes(size: usize) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let bytes = vec![b'b'; size];
    let blob = hash(&fixture.repo, "blob", &bytes);
    drop(bytes);
    assert_eq!(
        text(&fixture.repo, &["cat-file", "-s", &blob]),
        size.to_string()
    );
    let tree = checked_tree(&fixture.repo, &[("100644", b"bounded.bin", &blob)]);
    archive(&fixture, &tree);
    capture_bound(&fixture, size == 64 * 1024 * 1024, |history| {
        assert_eq!(
            history
                .objects
                .iter()
                .find(|o| o.oid == blob)
                .unwrap()
                .bytes
                .len(),
            size
        );
    });
}
#[test]
fn feature_red_blob_64m_inclusive() {
    blob_bytes(64 * 1024 * 1024);
}
#[test]
fn feature_red_blob_64m_plus_one_whole_refusal() {
    blob_bytes(64 * 1024 * 1024 + 1);
}

fn total_raw(size: usize) {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let old_total: usize = fixture
        .history
        .objects
        .iter()
        .map(|o| o.bytes.len())
        .chain(
            fixture
                .history
                .revisions
                .iter()
                .map(|r| r.original.as_ref().unwrap().bytes.len()),
        )
        .sum();
    let old_root = fixture.history.revisions.last().unwrap().root.clone();
    let root_len = git(&fixture.repo, &["cat-file", "tree", &old_root], b"").len();
    let prototype = commit(
        &fixture.repo,
        &old_root,
        &[&fixture.history.source_head],
        "",
        "bounds/unsupported fixture\n",
    );
    let commit_len = git(&fixture.repo, &["cat-file", "commit", &prototype], b"").len();
    let archive_len = 2 * (6 + 1 + 3 + 1 + 20);
    let payload = size - old_total - root_len - commit_len - archive_len;
    let first_size = 64 * 1024 * 1024;
    let second_size = payload - first_size;
    assert!(second_size <= 64 * 1024 * 1024);
    let first = hash(&fixture.repo, "blob", &vec![b'1'; first_size]);
    let second = hash(&fixture.repo, "blob", &vec![b'2'; second_size]);
    assert_ne!(first, second);
    let tree = checked_tree(
        &fixture.repo,
        &[("100644", b"one", &first), ("100644", b"two", &second)],
    );
    assert_eq!(
        git(&fixture.repo, &["cat-file", "tree", &tree], b"").len(),
        archive_len
    );
    archive(&fixture, &tree);
    let head = text(&fixture.repo, &["rev-parse", BOARD]);
    let root = text(&fixture.repo, &["rev-parse", &format!("{head}^{{tree}}")]);
    assert_eq!(
        git(&fixture.repo, &["cat-file", "tree", &root], b"").len(),
        root_len
    );
    assert_eq!(
        git(&fixture.repo, &["cat-file", "commit", &head], b"").len(),
        commit_len
    );
    assert_eq!(
        old_total + root_len + commit_len + archive_len + first_size + second_size,
        size
    );
    capture_bound(&fixture, size == 128 * 1024 * 1024, |history| {
        assert_eq!(
            history
                .objects
                .iter()
                .map(|o| o.bytes.len())
                .chain(
                    history
                        .revisions
                        .iter()
                        .map(|r| r.original.as_ref().unwrap().bytes.len())
                )
                .sum::<usize>(),
            size,
            "total includes unique raw blobs/trees AND commits"
        );
    });
}
#[test]
fn feature_red_total_raw_128m_inclusive() {
    total_raw(128 * 1024 * 1024);
}
#[test]
fn feature_red_total_raw_128m_plus_one_whole_refusal() {
    total_raw(128 * 1024 * 1024 + 1);
}

struct OwnedChild(Option<std::process::Child>);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            // SAFETY: kill only signals the child's own process group.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
        }
    }
}
fn child_handles(
    source: &Path,
    database: &Path,
    destination: &Path,
) -> quecto::composition::bootstrap::project_board_history::BoardHistoryImportHandles {
    import_handles(
        source,
        database,
        destination,
        quecto::domain::project_board::value_objects::telemetry::BoardId::of_hex(
            "0123456789abcdef",
        )
        .unwrap(),
    )
}
#[test]
fn feature_red_prepared_owned_child_crash_rolls_back_all_rows() {
    const CHILD: &str = "QUECTO_B3_PREPARED_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let path = std::path::PathBuf::from(path);
        let source =
            std::path::PathBuf::from(std::env::var_os("QUECTO_B3_PREPARED_SOURCE").unwrap());
        let destination =
            std::path::PathBuf::from(std::env::var_os("QUECTO_B3_PREPARED_DESTINATION").unwrap());
        let history = manifest(&source, HistoryAlgorithm::Sha1);
        let handles = child_handles(&source, &path, &destination);
        let _prepared = handles.store.prepare_import(&history).unwrap();
        std::fs::write(path.with_extension("ready"), b"prepared uncommitted").unwrap();
        loop {
            std::thread::park();
        }
    }
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let store = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    let before = fingerprint(&fixture.database);
    let handles = fixture.b_handles();
    // Deliberate direct parent result guard: baseline must fail on the missing
    // prepared feature, never on a missing child acknowledgement/setup timeout.
    let prepared = handles
        .store
        .prepare_import(&fixture.history)
        .expect("missing B3 prepared import (owned crash downstream not yet reached)");
    drop(prepared);
    assert_eq!(fingerprint(&fixture.database), before);
    let output = tempfile::tempfile().unwrap();
    let mut command = Command::new("timeout");
    command.args(["-s","KILL","20",std::env::current_exe().unwrap().to_str().unwrap(),"--exact","project_board_history_safety::feature_red_prepared_owned_child_crash_rolls_back_all_rows","--nocapture"]).env(CHILD,&fixture.database).env("QUECTO_B3_PREPARED_SOURCE",&fixture.repo).env("QUECTO_B3_PREPARED_DESTINATION",&fixture.destination).stdout(Stdio::from(output.try_clone().unwrap())).stderr(Stdio::from(output));
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: the pre-exec hook only calls async-signal-safe setsid and setrlimit.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::setrlimit(libc::RLIMIT_CORE, &limit) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
    }
    let mut child = OwnedChild(Some(command.spawn().unwrap()));
    let started = Instant::now();
    let ready = loop {
        if fixture.database.with_extension("ready").exists() {
            break true;
        }
        if started.elapsed() < Duration::from_secs(10)
            && child.0.as_mut().unwrap().try_wait().unwrap().is_none()
        {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        break false;
    };
    drop(child);
    assert!(
        ready,
        "future prepared implementation must open owned transaction within10s"
    );
    assert_eq!(
        fingerprint(&fixture.database),
        before,
        "kill/reap rolls back every new table/column/unreachable row"
    );
    assert_eq!(store.read(), Err(BoardStoreError::NoBoard));
    let integrity: String = rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

#[test]
fn feature_red_source_shallow_history_is_refused_without_hidden_fetch() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let boundary = fixture.history.revisions[5]
        .original
        .as_ref()
        .unwrap()
        .oid
        .clone();
    std::fs::write(fixture.repo.join("shallow"), format!("{boundary}\n")).unwrap();
    assert_eq!(
        text(&fixture.repo, &["rev-parse", "--is-shallow-repository"]),
        "true"
    );
    let refs = text(&fixture.repo, &["show-ref"]);
    let result = fixture.b_handles().source.capture();
    assert!(
        matches!(result, Err(HistoryError::Unsupported(_))),
        "complete history is required, not silently truncated/shallow: {result:?}"
    );
    assert_eq!(text(&fixture.repo, &["show-ref"]), refs);
    assert!(fixture.repo.join("shallow").exists());
    assert!(!fixture.database.exists());
}

#[test]
fn feature_red_snapshot_missing_replaced_schema_and_board_binding_fail_closed() {
    use quecto::domain::project_board::value_objects::telemetry::BoardId;
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let handles = fixture.b_handles();
    handles
        .import
        .execute()
        .expect("import establishes board binding for read-only snapshot refusal contracts");
    let before = fingerprint(&fixture.database);
    let snapshot = handles.store.snapshot().unwrap();
    let frontier = snapshot.frontier().unwrap();
    let other = import_handles(
        &fixture.repo,
        &fixture.database,
        &fixture.destination,
        BoardId::of_hex("fedcba9876543210").unwrap(),
    );
    assert!(
        matches!(other.store.snapshot(), Err(HistoryError::Conflict(_))),
        "read-only snapshot must verify non-null board binding"
    );
    assert_eq!(fingerprint(&fixture.database), before);
    drop(snapshot);
    let saved = fixture.database.with_extension("saved");
    std::fs::rename(&fixture.database, &saved).unwrap();
    assert!(matches!(
        handles.store.snapshot(),
        Err(HistoryError::Unavailable(_))
    ));
    assert!(
        !fixture.database.exists(),
        "read-only snapshot must not create missing DB"
    );
    let foreign = rusqlite::Connection::open(&fixture.database).unwrap();
    foreign
        .execute_batch("CREATE TABLE foreign_store (raw BLOB); PRAGMA user_version=900;")
        .unwrap();
    foreign.close().unwrap();
    let foreign_bytes = std::fs::read(&fixture.database).unwrap();
    assert!(
        matches!(
            handles.store.snapshot(),
            Err(HistoryError::Unsupported(_) | HistoryError::Conflict(_))
        ),
        "replaced foreign schema is refused, not initialized/migrated"
    );
    assert_eq!(std::fs::read(&fixture.database).unwrap(), foreign_bytes);
    assert_eq!(frontier, BoardHead("1:12".into()));
}

#[test]
fn feature_red_actual_import_proof_failure_rolls_back_all_rows_and_provenance() {
    use quecto::composition::bootstrap::project_board_history::build_board_history_import_from_ports;
    use std::sync::Arc;
    struct CorruptSource(LinearHistory);
    impl BoardHistorySource for CorruptSource {
        fn capture(&self) -> HistoryResult<LinearHistory> {
            Ok(self.0.clone())
        }
    }
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let native = SqliteProjectBoardStore::open(&fixture.database, writer()).unwrap();
    let before = fingerprint(&fixture.database);
    let mut damaged = fixture.history.clone();
    let object = damaged
        .objects
        .iter_mut()
        .find(|o| o.kind == HistoryKind::Blob && !o.bytes.is_empty())
        .unwrap();
    let original = object.bytes.clone();
    let last = object.bytes.len() - 1;
    object.bytes[last] ^= 1;
    assert_eq!(object.bytes.len(), original.len());
    assert_ne!(object.bytes, original);
    let actual = fixture.b_handles();
    let handles = build_board_history_import_from_ports(
        Arc::new(CorruptSource(damaged)),
        actual.store,
        actual.sink,
    );
    let result = handles.import.execute();
    assert!(
        matches!(result, Err(HistoryError::Malformed(_))),
        "application shared full export proof must detect mismatched original body/OID, not commit it: {result:?}"
    );
    assert_eq!(
        fingerprint(&fixture.database),
        before,
        "all imported rows,unreachable objects,epochs and provenance roll back"
    );
    assert_eq!(native.read(), Err(BoardStoreError::NoBoard));
    assert!(text(&fixture.destination, &["for-each-ref"]).is_empty());
}

#[test]
fn feature_red_import_schema3_normalized_history_and_source_provenance() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha256, 12);
    fixture.verify();
    let report = fixture
        .b_handles()
        .import
        .execute()
        .expect("schema3 import with normalized history");
    assert_eq!(report.source_head, fixture.history.source_head);
    let c = rusqlite::Connection::open(&fixture.database).unwrap();
    let version: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 4, "schema 4: B3 history and B7 op answers");
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for table in [
        "objects",
        "tree_entries",
        "state",
        "revisions",
        "git_objects",
        "history_meta",
        "epochs",
        "revision_principals",
        "op_results",
    ] {
        assert!(
            tables.iter().any(|name| name == table),
            "normalized schema4 includes {table}"
        );
    }
    let count: i64 = c
        .query_row("SELECT count(*) FROM revisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 12);
    let (source,remote,source_ref,imported):(String,String,String,i64)=c.query_row("SELECT source_head,source_remote,source_ref,imported_count FROM history_meta WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        (source, remote, source_ref, imported),
        (
            fixture.history.source_head,
            "origin".into(),
            BOARD.into(),
            12
        )
    );
}
