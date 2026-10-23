//! Reusable runtime characterization of metadata admission, not a second reader.
//! Future adapters run these bodies through their own raw fixture implementation.
use super::project_board_git_store::Remote;
use super::project_board_store::task;
use quecto::application::project_board::ports::{
    BoardArtifact, BoardHead, BoardStoreError, BoardWrite, ProjectBoardStore, SetAsideClear,
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
        Self {
            name,
            mode: "100644",
            bytes: vec![b'x'; size],
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
}
impl ReaderFixture for Remote {
    fn store(&self) -> Box<dyn ProjectBoardStore> {
        self.client(0, "Reader")
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
fn unchanged(f: &impl ReaderFixture, before: &str) {
    assert_eq!(f.fingerprint(), before);
}

/// Reads count every listed entry, even strays, before admitting any bytes.
fn check_counts<F: ReaderFixture>(make: impl Fn() -> F) {
    for (folder, cap, _, _) in FOLDERS {
        for extra in [0, 1] {
            let f = make();
            f.batch(&[(
                folder,
                (0..cap + extra)
                    .map(|i| Raw::file(format!("stray{i}.txt"), 0))
                    .collect(),
            )]);
            let store = f.store();
            assert_eq!(store.read().is_ok(), extra == 0, "{folder} {extra}");
            if extra == 1 {
                assert!(matches!(store.read(), Err(BoardStoreError::TooLarge(_))));
            }
            assert_eq!(store.read().is_ok(), extra == 0, "repeat {folder}");
        }
    }
    for extra in [0, 1] {
        let f = make();
        let folders: Vec<_> = ["tasks", "plans", "runs", "reviews", "changes"]
            .into_iter()
            .enumerate()
            .map(|(i, folder)| {
                let count = if i < 4 { 4096 } else { extra };
                (
                    folder,
                    (0..count)
                        .map(|n| Raw::file(format!("stray{n}.txt"), 0))
                        .collect(),
                )
            })
            .collect();
        f.batch(&folders);
        match extra {
            0 => assert_eq!(f.store().read().unwrap().quarantined.len(), 16384),
            1 => assert!(matches!(
                f.store().read(),
                Err(BoardStoreError::TooLarge(_))
            )),
            _ => unreachable!(),
        }
    }
}

/// Decode failures still consume admitted bytes. Metadata-only dispositions do not.
fn check_bytes<F: ReaderFixture>(make: impl Fn() -> F) {
    for (folder, _, budget, cap) in FOLDERS {
        for extra in [0, 1] {
            let f = make();
            f.batch(&[(folder, bytes(folder, budget + extra, cap))]);
            let result = f.store().read();
            match extra {
                0 => {
                    let board = result.unwrap();
                    assert_eq!(board.quarantined.len(), budget.div_ceil(cap));
                    let before = f.fingerprint();
                    let write =
                        f.store()
                            .write(&board.head, &[artifact(folder)], "board: over bytes");
                    assert!(
                        matches!(write, Err(BoardStoreError::WriteTooLarge(_))),
                        "{folder}: {write:?}"
                    );
                    unchanged(&f, &before);
                    let file = &board.quarantined[0];
                    written(f.store().clear_set_aside(
                        &board.head,
                        &clear(&file.path, &file.object),
                        "board: clear",
                    ));
                    assert_eq!(
                        f.store().read().unwrap().quarantined.len(),
                        board.quarantined.len() - 1
                    );
                }
                1 => {
                    assert!(
                        matches!(result, Err(BoardStoreError::TooLarge(_))),
                        "{folder}: {result:?}"
                    );
                    let before = f.fingerprint();
                    assert!(matches!(
                        f.store()
                            .write(&f.head(), &[artifact(folder)], "board: over"),
                        Err(BoardStoreError::TooLarge(_))
                    ));
                    assert!(matches!(
                        f.store().clear_set_aside(
                            &f.head(),
                            &clear(&format!("{folder}/{}", name(folder, 0)), &"0".repeat(40)),
                            "board: clear"
                        ),
                        Err(BoardStoreError::TooLarge(_))
                    ));
                    unchanged(&f, &before);
                }
                _ => unreachable!(),
            }
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
        let result = f.store().read();
        match extra {
            0 => {
                let board = result.unwrap();
                let before = f.fingerprint();
                assert!(matches!(
                    f.store()
                        .write(&board.head, &[artifact("reviews")], "board: over"),
                    Err(BoardStoreError::WriteTooLarge(_))
                ));
                unchanged(&f, &before);
                let file = &board.quarantined[0];
                written(f.store().clear_set_aside(
                    &board.head,
                    &clear(&file.path, &file.object),
                    "board: clear",
                ));
                assert!(f.store().read().is_ok());
            }
            1 => {
                assert!(matches!(result, Err(BoardStoreError::TooLarge(_))));
                let before = f.fingerprint();
                assert!(matches!(
                    f.store()
                        .write(&f.head(), &[artifact("reviews")], "board: over"),
                    Err(BoardStoreError::TooLarge(_))
                ));
                assert!(matches!(
                    f.store().clear_set_aside(
                        &f.head(),
                        &clear("tasks/a0.md", &"0".repeat(40)),
                        "board: clear"
                    ),
                    Err(BoardStoreError::TooLarge(_))
                ));
                unchanged(&f, &before);
            }
            _ => unreachable!(),
        }
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
                let file = &board.quarantined[0];
                written(store.clear_set_aside(
                    &head,
                    &clear(&file.path, &file.object),
                    "board: clear",
                ));
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
        f.batch(&[(
            "format",
            vec![Raw {
                name: "board".into(),
                mode: "100644",
                bytes: marker.into(),
            }],
        )]);
        let store = f.store();
        assert_eq!(f.observed_head(), Some(f.head().0));
        assert!(matches!(store.read(), Err(BoardStoreError::Unavailable(_))));
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
                Raw {
                    name: "a.md".into(),
                    mode: "100644",
                    bytes: canonical,
                },
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
    let a = board
        .quarantined
        .iter()
        .find(|q| q.path == "tasks/a.json")
        .unwrap();
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
    assert_eq!(
        after
            .quarantined
            .iter()
            .find(|q| q.path == "tasks/b.md")
            .unwrap()
            .object,
        keep
    );
    let opaque = board
        .quarantined
        .iter()
        .find(|q| q.path == "tasks/opaque.md")
        .unwrap();
    assert_eq!(
        after
            .quarantined
            .iter()
            .find(|q| q.path == opaque.path)
            .unwrap()
            .object,
        opaque.object
    );
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
    let f = fresh();
    f.batch(&[(
        "format",
        vec![Raw {
            name: "board".into(),
            mode: "100644",
            bytes: b"quecto-board 2\n".to_vec(),
        }],
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
        let file = &board.quarantined[0];
        written(f.store().clear_set_aside(
            &board.head,
            &clear(&file.path, &file.object),
            "board: clear",
        ));
        assert_eq!(f.store().read().unwrap().quarantined.len(), 16382 + extra);
    }
}
#[test]
fn project_board_reader_whole_board_write_clear_counts() {
    check_board_mutations(fresh);
}
