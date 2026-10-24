//! Reusable runtime characterization of metadata admission, not a second reader.
//! Future adapters run these bodies through their own raw fixture implementation.
use super::project_board_git_store::Remote;
use super::project_board_store::task;
use quecto::application::project_board::ports::{
    BoardArtifact, BoardHead, BoardSnapshot, BoardStoreError, BoardWrite, ProjectBoardStore,
    QuarantinedFile, SetAsideClear,
};
use std::collections::BTreeMap;

const MIB: usize = 1024 * 1024;
const FOLDERS: [(&str, usize, usize, usize); 6] = [
    ("tasks", 4096, 16 * MIB, 512 * 1024),
    ("plans", 4096, 24 * MIB, 1536 * 1024),
    ("runs", 4096, 16 * MIB, 768 * 1024),
    ("changes", 4096, 4 * MIB, 8 * 1024),
    ("reviews", 4096, 16 * MIB, MIB),
    ("discussions", 2048, 16 * MIB, MIB),
];

/// One raw entry. Repeated names intentionally remain repeated.
struct Raw {
    name: String,
    mode: &'static str,
    bytes: Vec<u8>,
}
impl Raw {
    fn file(name: String, size: usize) -> Self {
        Self::encoded(name, vec![b'x'; size])
    }
    fn encoded(name: String, bytes: Vec<u8>) -> Self {
        Self {
            name,
            mode: "100644",
            bytes,
        }
    }
}

/// A raw fixture is test plumbing only; acceptance stays in the real store.
trait ReaderFixture {
    fn store(&self) -> Box<dyn ProjectBoardStore>;
    fn batch(&self, folders: &[(&str, Vec<Raw>)]);
    fn head(&self) -> BoardHead;
    fn fingerprint(&self) -> String;
    fn observed_head(&self) -> Option<String>;
    fn object_size(&self, oid: &str) -> usize;
}
impl ReaderFixture for Remote {
    fn store(&self) -> Box<dyn ProjectBoardStore> {
        self.client(0, "Reader")
    }
    fn object_size(&self, oid: &str) -> usize {
        self.raw_object_size(oid)
    }
    fn head(&self) -> BoardHead {
        BoardHead(self.raw_head())
    }
    fn fingerprint(&self) -> String {
        self.raw_fingerprint()
    }
    fn observed_head(&self) -> Option<String> {
        self.client(0, "Reader").head().unwrap()
    }
    fn batch(&self, folders: &[(&str, Vec<Raw>)]) {
        self.raw_batch(
            folders
                .iter()
                .map(|(folder, rows)| {
                    (
                        *folder,
                        rows.iter()
                            .map(|r| (r.name.as_str(), r.mode, r.bytes.as_slice()))
                            .collect(),
                    )
                })
                .collect(),
        );
    }
}
fn fresh() -> Remote {
    let fixture = Remote::new();
    fixture.store().bootstrap().unwrap();
    fixture
}
fn name(folder: &str, i: usize) -> String {
    match folder {
        "tasks" | "discussions" => format!("a{i}.md"),
        _ => format!("a{i}.1.md"),
    }
}
fn bytes(folder: &str, total: usize, cap: usize) -> Vec<Raw> {
    assert!(cap > 0);
    (0..total.div_ceil(cap))
        .map(|i| Raw::file(name(folder, i), (total - i * cap).min(cap)))
        .collect()
}
fn clear(path: &str, object: &str) -> SetAsideClear {
    SetAsideClear {
        path: path.into(),
        object: object.into(),
        repeated: BTreeMap::new(),
    }
}
fn artifact(folder: &str) -> BoardArtifact {
    use quecto::infrastructure::test_support::board_artifacts::{
        change, discussion, plan, review, run,
    };
    match folder {
        "tasks" => BoardArtifact::Task(task("new", "New")),
        "plans" => BoardArtifact::Plan(plan("new", 1)),
        "runs" => BoardArtifact::Run(run("new", 1)),
        "changes" => BoardArtifact::Change(change("new", 1)),
        "reviews" => BoardArtifact::Review(review("new", 1)),
        "discussions" => BoardArtifact::Discussion(discussion("new")),
        _ => panic!("unsupported folder"),
    }
}
fn written(result: Result<BoardWrite, BoardStoreError>) {
    assert!(matches!(result, Ok(BoardWrite::Written(_))), "{result:?}");
}
fn clear_first(store: &dyn ProjectBoardStore, board: &BoardSnapshot) {
    let file = &board.quarantined[0];
    written(store.clear_set_aside(
        &board.head,
        &clear(&file.path, &file.object),
        "board: clear",
    ));
}
fn aside<'a>(board: &'a BoardSnapshot, path: &str) -> &'a QuarantinedFile {
    board.quarantined.iter().find(|q| q.path == path).unwrap()
}
fn unchanged(f: &impl ReaderFixture, before: &str) {
    assert_eq!(f.fingerprint(), before);
}

/// Reads count every listed entry, even strays, before admitting any bytes.
fn check_counts<F: ReaderFixture>(make: impl Fn() -> F) {
    let cases = FOLDERS.map(|(folder, cap, _, _)| vec![(folder, cap)]);
    let aggregate = vec![
        ("tasks", 4096),
        ("plans", 4096),
        ("runs", 4096),
        ("reviews", 4096),
        ("changes", 0),
    ];
    for case in cases.into_iter().chain([aggregate]) {
        for extra in [0, 1] {
            let f = make();
            let mut counts = case.clone();
            counts.last_mut().unwrap().1 += extra;
            let folders: Vec<_> = counts
                .into_iter()
                .map(|(folder, count)| {
                    (
                        folder,
                        (0..count)
                            .map(|i| Raw::file(format!("stray{i}.txt"), 0))
                            .collect(),
                    )
                })
                .collect();
            f.batch(&folders);
            let store = f.store();
            assert_eq!(store.read().is_ok(), extra == 0, "{case:?} {extra}");
            if extra == 1 {
                assert!(matches!(store.read(), Err(BoardStoreError::TooLarge(_))));
            } else if case.len() > 1 {
                assert_eq!(store.read().unwrap().quarantined.len(), 16384);
            }
            assert_eq!(store.read().is_ok(), extra == 0, "repeat {case:?}");
        }
    }
}

/// The same read/write/clear boundary contract applies per folder and board-wide.
/// A folder has an exact quarantine count; the aggregate case checks readable state.
fn check_byte_boundary(f: &impl ReaderFixture, folder: &str, extra: usize, count: Option<usize>) {
    let result = f.store().read();
    let before = f.fingerprint();
    match extra {
        0 => {
            let board = result.unwrap();
            if let Some(count) = count {
                assert_eq!(board.quarantined.len(), count);
            }
            assert!(matches!(
                f.store()
                    .write(&board.head, &[artifact(folder)], "board: over bytes"),
                Err(BoardStoreError::WriteTooLarge(_))
            ));
            unchanged(f, &before);
            clear_first(f.store().as_ref(), &board);
            let after = f.store().read().unwrap();
            if count.is_some() {
                assert_eq!(after.quarantined.len(), board.quarantined.len() - 1);
            }
        }
        1 => {
            assert!(
                matches!(result, Err(BoardStoreError::TooLarge(_))),
                "{folder}: {result:?}"
            );
            assert!(matches!(
                f.store()
                    .write(&f.head(), &[artifact(folder)], "board: over"),
                Err(BoardStoreError::TooLarge(_))
            ));
            let clear_folder = if count.is_some() { folder } else { "tasks" };
            assert!(matches!(
                f.store().clear_set_aside(
                    &f.head(),
                    &clear(
                        &format!("{clear_folder}/{}", name(clear_folder, 0)),
                        &"0".repeat(40)
                    ),
                    "board: clear"
                ),
                Err(BoardStoreError::TooLarge(_))
            ));
            unchanged(f, &before);
        }
        _ => unreachable!(),
    }
}

/// Decode failures still consume admitted bytes. Metadata-only dispositions do not.
fn check_bytes<F: ReaderFixture>(make: impl Fn() -> F) {
    for (folder, _, budget, cap) in FOLDERS {
        for extra in [0, 1] {
            let f = make();
            f.batch(&[(folder, bytes(folder, budget + extra, cap))]);
            check_byte_boundary(&f, folder, extra, Some(budget.div_ceil(cap)));
        }
        let f = make();
        let mut rows = bytes(folder, budget, cap);
        rows.push(Raw::file(name(folder, 9000), cap + 1));
        rows.push(Raw {
            mode: "100755",
            ..Raw::file(name(folder, 9001), cap)
        });
        rows.push(Raw::file("stray.txt".into(), cap));
        rows.push(Raw::file(name(folder, 9002), cap));
        rows.push(Raw::file(name(folder, 9002), cap));
        f.batch(&[(folder, rows)]);
        let board = f.store().read().unwrap();
        assert_eq!(
            board.quarantined.len(),
            budget.div_ceil(cap) + 5,
            "{folder}"
        );
        assert!(board.tasks.is_empty());
    }
    for extra in [0, 1] {
        let f = make();
        f.batch(&[
            ("tasks", bytes("tasks", 16 * MIB, 512 * 1024)),
            ("plans", bytes("plans", 24 * MIB, 1536 * 1024)),
            ("runs", bytes("runs", 16 * MIB, 768 * 1024)),
            ("reviews", bytes("reviews", 8 * MIB + extra, MIB)),
        ]);
        check_byte_boundary(&f, "reviews", extra, None);
    }
}

/// The 2048 task write cap differs from the 4096 read cap. Clears do not grow it.
fn check_task_mutations<F: ReaderFixture>(make: impl Fn() -> F) {
    for count in [2047, 2048, 4096, 4097] {
        let f = make();
        f.batch(&[(
            "tasks",
            (0..count).map(|i| Raw::file(name("tasks", i), 1)).collect(),
        )]);
        let store = f.store();
        let before = f.fingerprint();
        let result = store.write_tasks(&f.head(), &[task("new", "New")], "board: new");
        match count {
            2047 => {
                written(result);
                assert_eq!(store.read().unwrap().quarantined.len(), 2047);
            }
            2048 | 4096 => {
                assert!(
                    matches!(result, Err(BoardStoreError::InvalidWrite(_))),
                    "{result:?}"
                );
                unchanged(&f, &before);
            }
            4097 => {
                assert!(
                    matches!(result, Err(BoardStoreError::TooLarge(_))),
                    "{result:?}"
                );
                unchanged(&f, &before);
            }
            _ => unreachable!(),
        }
        let head = f.head();
        match store.read() {
            Ok(board) => {
                assert_eq!(head, board.head);
                clear_first(store.as_ref(), &board);
                assert_eq!(store.read().unwrap().quarantined.len(), count - 1);
            }
            Err(BoardStoreError::TooLarge(_)) => {
                let result = store.clear_set_aside(
                    &head,
                    &clear("tasks/a0.md", &"0".repeat(40)),
                    "board: clear",
                );
                assert!(matches!(result, Err(BoardStoreError::TooLarge(_))));
                unchanged(&f, &before);
            }
            other => panic!("{other:?}"),
        }
    }
}

/// Neither mutation can rewrite damaged/unknown formats, even when the head exists.
fn check_heads<F: ReaderFixture>(make: impl Fn() -> F) {
    for marker in [b"quecto-board 999\n".as_slice(), b"broken".as_slice()] {
        let f = make();
        f.batch(&[("format", vec![Raw::encoded("board".into(), marker.into())])]);
        let store = f.store();
        assert_eq!(f.observed_head(), Some(f.head().0));
        assert!(matches!(store.read(), Err(BoardStoreError::Unavailable(_))));
        let before = f.fingerprint();
        assert!(matches!(
            store.write_tasks(&f.head(), &[task("a", "A")], "board: a"),
            Err(BoardStoreError::Unavailable(_))
        ));
        unchanged(&f, &before);
        assert!(matches!(
            store.clear_set_aside(
                &f.head(),
                &clear("tasks/a.md", &"0".repeat(40)),
                "board: clear"
            ),
            Err(BoardStoreError::Unavailable(_))
        ));
        unchanged(&f, &before);
    }
}

#[test]
fn project_board_reader_counts_exact_and_over() {
    check_counts(fresh);
}
#[test]
fn project_board_reader_bytes_exact_over_and_admission() {
    check_bytes(fresh);
}
#[test]
fn project_board_reader_task_read_write_clear_caps() {
    check_task_mutations(fresh);
}
#[test]
fn project_board_reader_rejected_heads_preserve_raw_state() {
    check_heads(fresh);
}

/// Legacy siblings suppress a valid task, opaque files survive, and repeated
/// copies require an explicit complete resolution, never an arbitrary winner.
fn check_quarantine<F: ReaderFixture>(make: impl Fn() -> F) {
    use quecto::application::project_board::ports::KeepCopy;
    use quecto::infrastructure::persistence::project_board::artifacts::file_of;
    let f = make();
    let canonical = file_of(&BoardArtifact::Task(task("a", "A"))).unwrap();
    f.batch(&[
        ("archive", vec![Raw::file("opaque.md".into(), 7)]),
        (
            "tasks",
            vec![
                Raw::encoded("a.md".into(), canonical),
                Raw::file("a.json".into(), 1),
                Raw::file("b.md".into(), 1),
                Raw::file("b.md".into(), 2),
                Raw::file("opaque.md".into(), 3),
            ],
        ),
    ]);
    let store = f.store();
    let board = store.read().unwrap();
    assert!(board.tasks.is_empty(), "legacy sibling suppresses a.md");
    assert_eq!(board.quarantined.len(), 5);
    assert!(board.quarantined.iter().filter(|q| ["tasks/a.md", "tasks/a.json", "tasks/opaque.md"].contains(&q.path.as_str())).all(|q| q.carried));
    assert!(
        board
            .quarantined
            .iter()
            .filter(|q| q.path == "tasks/b.md")
            .all(|q| !q.carried)
    );
    let before = f.fingerprint();
    assert!(matches!(
        store.write_tasks(&board.head, &[task("new", "New")], "board: new"),
        Ok(BoardWrite::Refused(_))
    ));
    unchanged(&f, &before);
    let a = aside(&board, "tasks/a.json");
    let copies: std::collections::BTreeSet<_> = board
        .quarantined
        .iter()
        .filter(|q| q.path == "tasks/b.md")
        .map(|q| q.object.clone())
        .collect();
    assert_eq!(copies.len(), 2);
    let mut clearing = clear(&a.path, &a.object);
    assert!(matches!(
        store.clear_set_aside(&board.head, &clearing, "board: clear"),
        Ok(BoardWrite::Refused(_))
    ));
    unchanged(&f, &before);
    let keep = copies.iter().next().unwrap().clone();
    clearing.repeated.insert(
        "tasks/b.md".into(),
        KeepCopy::Object {
            keep: keep.clone(),
            seen: copies,
        },
    );
    written(store.clear_set_aside(&board.head, &clearing, "board: resolve"));
    let after = store.read().unwrap();
    assert_eq!(after.tasks, [task("a", "A")]);
    assert_eq!(after.quarantined.len(), 2);
    assert_eq!(aside(&after, "tasks/b.md").object, keep);
    let opaque = aside(&board, "tasks/opaque.md");
    assert_eq!(aside(&after, &opaque.path).object, opaque.object);
    // Root-preserving raw batches and both accepted mutations retain archive.
    let archive = before
        .lines()
        .find(|row| row.ends_with("\tarchive"))
        .unwrap();
    assert!(f.fingerprint().lines().any(|row| row == archive));
    written(store.write_tasks(&after.head, &[task("new", "New")], "board: new"));
    assert!(f.fingerprint().lines().any(|row| row == archive));
}
#[test]
fn project_board_reader_quarantine_legacy_opaque_and_repeated_copies() {
    check_quarantine(fresh);
}

#[test]
fn project_board_reader_legacy_format_is_read_and_upgraded() {
    check_legacy_format(fresh);
}
fn check_legacy_format<F: ReaderFixture>(make: impl Fn() -> F) {
    let f = make();
    f.batch(&[(
        "format",
        vec![Raw::encoded("board".into(), b"quecto-board 2\n".to_vec())],
    )]);
    let store = f.store();
    let board = store.read().unwrap();
    written(store.write_tasks(&board.head, &[task("a", "A")], "board: upgrade"));
    assert_eq!(store.read().unwrap().tasks, [task("a", "A")]);
}

fn check_board_mutations<F: ReaderFixture>(make: impl Fn() -> F) {
    for extra in [0, 1, 2] {
        let f = make();
        let folders: Vec<_> = [
            ("tasks", 2048),
            ("plans", 4096),
            ("runs", 4096),
            ("reviews", 4096),
            ("discussions", 2047),
            ("changes", extra),
        ]
        .into_iter()
        .map(|(folder, count)| {
            (
                folder,
                (0..count).map(|i| Raw::file(name(folder, i), 1)).collect(),
            )
        })
        .collect();
        f.batch(&folders);
        let before = f.fingerprint();
        let result = f
            .store()
            .write(&f.head(), &[artifact("changes")], "board: count");
        match extra {
            0 => written(result),
            1 => {
                assert!(matches!(result, Err(BoardStoreError::InvalidWrite(_))));
                unchanged(&f, &before);
            }
            2 => {
                assert!(matches!(result, Err(BoardStoreError::TooLarge(_))));
                assert!(matches!(
                    f.store().clear_set_aside(
                        &f.head(),
                        &clear("tasks/a0.md", &"0".repeat(40)),
                        "board: clear"
                    ),
                    Err(BoardStoreError::TooLarge(_))
                ));
                unchanged(&f, &before);
                continue;
            }
            _ => unreachable!(),
        }
        let board = f.store().read().unwrap();
        clear_first(f.store().as_ref(), &board);
        assert_eq!(f.store().read().unwrap().quarantined.len(), 16382 + extra);
    }
}
#[test]
fn project_board_reader_whole_board_write_clear_counts() {
    check_board_mutations(fresh);
}

fn check_folder_writes<F: ReaderFixture>(make: impl Fn() -> F) {
    for (folder, cap, _, _) in FOLDERS.into_iter().skip(1) {
        for count in [cap - 1, cap] {
            let f = make();
            f.batch(&[(
                folder,
                (0..count).map(|i| Raw::file(name(folder, i), 1)).collect(),
            )]);
            let board = f.store().read().unwrap();
            assert_eq!(board.quarantined.len(), count);
            let before = f.fingerprint();
            let result = f
                .store()
                .write(&board.head, &[artifact(folder)], "board: count");
            match count == cap {
                true => {
                    assert!(
                        matches!(result, Err(BoardStoreError::InvalidWrite(_))),
                        "{folder}: {result:?}"
                    );
                    unchanged(&f, &before);
                }
                false => written(result),
            }
            let board = f.store().read().unwrap();
            let file = &board.quarantined[0];
            written(f.store().clear_set_aside(
                &board.head,
                &clear(&file.path, &file.object),
                "board: clear",
            ));
            assert_eq!(f.store().read().unwrap().quarantined.len(), count - 1);
        }
    }
}
#[test]
fn project_board_reader_folder_write_and_clear_counts() {
    check_folder_writes(fresh);
}

/// An absent expected revision is invalid, not a known stale revision (Lost).
fn check_unknown_heads<F: ReaderFixture>(make: impl Fn() -> F, absent: impl Fn(&F, &str) -> bool) {
    let f = make();
    let stale = f.head();
    f.batch(&[("tasks", vec![Raw::file("broken.md".into(), 1)])]);
    let board = f.store().read().unwrap();
    assert_eq!(board.quarantined.len(), 1);
    let file = aside(&board, "tasks/broken.md");
    let clearing = clear(&file.path, &file.object);
    let before = f.fingerprint();
    for expected in ["a".repeat(40), "b".repeat(64), "not-a-head".into()] {
        if [40, 64].contains(&expected.len()) {
            assert!(absent(&f, &expected), "actual backend contains {expected}");
        }
        assert_ne!(expected, board.head.0);
        assert_ne!(expected, stale.0);
        let head = BoardHead(expected);
        for writing in [true, false] {
            let result = match writing {
                true => f
                    .store()
                    .write(&head, &[artifact("tasks")], "board: valid write"),
                false => f
                    .store()
                    .clear_set_aside(&head, &clearing, "board: valid clear"),
            };
            assert!(
                matches!(result, Err(BoardStoreError::InvalidWrite(_))),
                "{result:?}"
            );
            assert_eq!(f.observed_head(), Some(board.head.0.clone()));
            unchanged(&f, &before);
        }
    }
    // Positive controls: these same operations are valid at the actual current head.
    written(
        f.store()
            .write(&board.head, &[artifact("tasks")], "board: valid write"),
    );
    let current = f.store().read().unwrap();
    written(
        f.store()
            .clear_set_aside(&current.head, &clearing, "board: valid clear"),
    );
    assert!(f.store().read().unwrap().quarantined.is_empty());
}
#[test]
fn project_board_reader_unknown_and_malformed_expected_heads() {
    check_unknown_heads(fresh, Remote::object_absent);
}

/// Pad only admitted regular canonical paths; board scope leaves review room.
fn prospective_rows(scope: usize, growth: usize, extra: usize) -> Vec<(&'static str, Vec<Raw>)> {
    let (folder, _, budget, cap) = FOLDERS[scope.min(5)];
    let (folder, budget, cap) = match scope {
        0..=5 => (folder, budget, cap),
        6 => ("reviews", 8 * MIB, MIB),
        _ => panic!("unsupported byte scope"),
    };
    assert!(growth <= cap && growth <= budget);
    let rows = bytes(folder, budget - growth + extra, cap);
    assert_eq!(
        rows.iter().map(|row| row.bytes.len()).sum::<usize>(),
        budget - growth + extra
    );
    let mut folders = vec![(folder, rows)];
    if scope == 6 {
        folders.extend([
            ("tasks", bytes("tasks", 16 * MIB, 512 * 1024)),
            ("plans", bytes("plans", 24 * MIB, 1536 * 1024)),
            ("runs", bytes("runs", 16 * MIB, 768 * 1024)),
        ]);
    }
    folders
}

/// Existing admitted bytes fit; only this mutation activates the final bytes.
fn check_prospective_bytes<F: ReaderFixture>(make: impl Fn() -> F, keeping: bool) {
    use quecto::application::project_board::ports::KeepCopy;
    use quecto::infrastructure::persistence::project_board::artifacts::file_of;
    for (scope, folder) in FOLDERS
        .iter()
        .map(|row| row.0)
        .chain(["reviews"])
        .enumerate()
    {
        let new = artifact(folder);
        let growth = if keeping {
            4096
        } else {
            file_of(&new).unwrap().len()
        };
        for extra in [0, 1] {
            let f = make();
            let mut folders = prospective_rows(scope, growth, extra);
            let repeated = name(folder, 9000);
            let path = format!("{folder}/{repeated}");
            if keeping {
                // Both copies are regular, under the file cap, and distinct objects/sizes.
                folders[0].1.extend([
                    Raw::file(repeated.clone(), growth),
                    Raw::file(repeated, growth - 1),
                ]);
            }
            f.batch(&folders);
            let board = f.store().read().unwrap();
            let before = f.fingerprint();
            let result = match keeping {
                true => {
                    let copies: Vec<_> = board
                        .quarantined
                        .iter()
                        .filter(|q| q.path == path)
                        .collect();
                    assert_eq!(copies.len(), 2);
                    assert!(copies.iter().all(|q| !q.carried));
                    assert_ne!(copies[0].object, copies[1].object);
                    let keep = copies
                        .iter()
                        .find(|q| f.object_size(&q.object) == growth)
                        .unwrap()
                        .object
                        .clone();
                    let mut clearing = clear(&path, &keep);
                    clearing.repeated.insert(
                        path.clone(),
                        KeepCopy::Object {
                            keep,
                            seen: copies.iter().map(|q| q.object.clone()).collect(),
                        },
                    );
                    f.store()
                        .clear_set_aside(&board.head, &clearing, "board: keep copy")
                }
                false => f.store().write(
                    &board.head,
                    std::slice::from_ref(&new),
                    "board: exact bytes",
                ),
            };
            match extra {
                0 => {
                    written(result);
                    let after = f.store().read().unwrap();
                    assert_ne!(after.head, board.head);
                    if keeping {
                        assert_eq!(f.object_size(&aside(&after, &path).object), growth);
                        assert!(
                            board
                                .quarantined
                                .iter()
                                .any(|q| q.path == path && q.object == aside(&after, &path).object)
                        );
                        assert_eq!(after.quarantined.len(), board.quarantined.len() - 1);
                    }
                }
                1 => {
                    let Err(BoardStoreError::WriteTooLarge(why)) = result else {
                        panic!("scope {scope}, keeping {keeping}: {result:?}");
                    };
                    assert!(why.contains("bytes"), "{why}");
                    if keeping {
                        assert!(why.contains(&path), "{why}");
                    }
                    assert_eq!(f.observed_head(), Some(board.head.0));
                    unchanged(&f, &before);
                }
                _ => unreachable!(),
            }
        }
    }
}
#[test]
fn project_board_reader_prospective_write_bytes_exact_and_over() {
    check_prospective_bytes(fresh, false);
}
#[test]
fn project_board_reader_prospective_kept_copy_bytes_exact_and_over() {
    check_prospective_bytes(fresh, true);
}

// The same full runtime bodies above, now on real normalized SQLite storage.
// Raw setup carries every untouched root child, including FORMAT and archive.
impl ReaderFixture for super::project_board_sqlite::Database {
    fn store(&self) -> Box<dyn ProjectBoardStore> {
        Box::new(self.client())
    }
    fn head(&self) -> BoardHead {
        self.client().fixture_head().unwrap()
    }
    fn fingerprint(&self) -> String {
        self.client().fixture_fingerprint().unwrap()
    }
    fn observed_head(&self) -> Option<String> {
        self.client().fixture_head().ok().map(|head| head.0)
    }
    fn object_size(&self, oid: &str) -> usize {
        self.client().fixture_object_size(oid).unwrap()
    }
    fn batch(&self, folders: &[(&str, Vec<Raw>)]) {
        use quecto::infrastructure::persistence::sqlite::project_board::RawEntry;
        self.client()
            .fixture_edit(|editor| {
                let (_, root) = editor.current()?;
                let mut roots = editor.entries(&root)?;
                for (folder, rows) in folders {
                    let files = rows
                        .iter()
                        .map(|row| {
                            Ok(RawEntry {
                                mode: row.mode.into(),
                                name: row.name.as_bytes().to_vec(),
                                oid: editor.blob(&row.bytes)?,
                            })
                        })
                        .collect::<Result<Vec<_>, BoardStoreError>>()?;
                    let tree = editor.tree(&files)?;
                    roots.retain(|row| row.name != folder.as_bytes());
                    roots.push(RawEntry {
                        mode: "040000".into(),
                        name: folder.as_bytes().to_vec(),
                        oid: tree,
                    });
                }
                editor.tree(&roots)
            })
            .unwrap();
    }
}
fn fresh_sqlite() -> super::project_board_sqlite::Database {
    let fixture = super::project_board_sqlite::Database::new();
    fixture.store().bootstrap().unwrap();
    fixture
}
#[cfg(test)]
mod sqlite {
    use super::*;
    #[test]
    fn counts_exact_and_over() {
        check_counts(fresh_sqlite);
    }
    #[test]
    fn bytes_exact_over_and_admission() {
        check_bytes(fresh_sqlite);
    }
    #[test]
    fn task_read_write_clear_caps() {
        check_task_mutations(fresh_sqlite);
    }
    #[test]
    fn rejected_heads_preserve_raw_state() {
        check_heads(fresh_sqlite);
    }
    #[test]
    fn quarantine_legacy_opaque_and_repeated_copies() {
        check_quarantine(fresh_sqlite);
    }
    #[test]
    fn legacy_format_is_read_and_upgraded() {
        check_legacy_format(fresh_sqlite);
    }
    #[test]
    fn whole_board_write_clear_counts() {
        check_board_mutations(fresh_sqlite);
    }
    #[test]
    fn folder_write_and_clear_counts() {
        check_folder_writes(fresh_sqlite);
    }
    #[test]
    fn unknown_and_malformed_expected_heads() {
        check_unknown_heads(fresh_sqlite, |f, head| {
            f.client().fixture_object_absent(head).unwrap()
        });
    }
    #[test]
    fn prospective_write_bytes_exact_and_over() {
        check_prospective_bytes(fresh_sqlite, false);
    }
    #[test]
    fn prospective_kept_copy_bytes_exact_and_over() {
        check_prospective_bytes(fresh_sqlite, true);
    }
}
