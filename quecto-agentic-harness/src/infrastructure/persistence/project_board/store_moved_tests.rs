//! A board fenced at format 4 (epic #2705 B13): every read and every write
//! answers that it moved, naming the URLs `format/MOVED` gives, and a write
//! makes no commit and pushes nothing. A `MOVED` quecto would not write is
//! refused as malformed, never written either. Fencing builds the fence's
//! tree, and fencing a fenced board to the same place is a no-op.
use super::*;
use crate::domain::project_board::value_objects::board_moved::{BoardMoved, MAX_MOVED_BYTES};
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::infrastructure::persistence::project_board::board_tree::MOVED_FORMAT;
use crate::infrastructure::persistence::project_board::store::Fence;

const MOVED: &[u8] =
    b"board_id 0123456789abcdef\nurl https://board.example:7443\nurl http://100.64.0.2:7443\n";

fn moved_to(urls: &[&str]) -> BoardMoved {
    let id = BoardId::of_hex("0123456789abcdef").unwrap();
    BoardMoved::new(id, urls.iter().map(|url| (*url).to_string()).collect()).unwrap()
}

fn moved() -> BoardMoved {
    moved_to(&["https://board.example:7443", "http://100.64.0.2:7443"])
}

/// The commits `repo` holds, reachable or not.
fn commits(repo: &Path) -> usize {
    let args = [
        "cat-file",
        "--batch-all-objects",
        "--batch-check=%(objecttype)",
    ];
    sh(repo, &args)
        .lines()
        .filter(|kind| *kind == "commit")
        .count()
}

/// The board fenced by hand in alice's clone on top of `before`, as the
/// cut-over's fence commit leaves it, and pushed.
fn fenced_by_hand(repos: &Repos, before: &BoardHead) -> String {
    let alice = &repos.alice;
    let marker = blob_in(alice, MOVED_FORMAT);
    let file = blob_in(alice, MOVED);
    let format = raw_rows(
        alice,
        &[("100644", "MOVED", file), ("100644", "board", marker)],
    );
    let root: String = sh(alice, &["ls-tree", &before.0])
        .lines()
        .filter(|row| !row.ends_with("\tformat"))
        .map(|row| format!("{row}\n"))
        .chain([format!("040000 tree {format}\tformat\n")])
        .collect();
    let tree = sh_with(alice, &["mktree"], root.as_bytes());
    let fence = sh(
        alice,
        &["commit-tree", &tree, "-p", &before.0, "-m", "board: fence"],
    );
    sh(
        alice,
        &["push", "-q", "origin", &format!("{fence}:{BOARD_REF}")],
    );
    fence
}

fn clear_of_a() -> crate::application::project_board::ports::SetAsideClear {
    crate::application::project_board::ports::SetAsideClear {
        path: "tasks/a.md".into(),
        object: "0".repeat(40),
        repeated: Default::default(),
    }
}

#[test]
fn a_moved_board_answers_board_moved_on_every_read_and_write_and_writes_nothing() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    let before = written(alice.write_tasks(&head, &[task("a")], "board: add a"));
    let fence = fenced_by_hand(&repos, &before);
    let bob = store(&repos.bob);
    let is_moved = |result: &dyn std::fmt::Debug, error: Option<&BoardStoreError>| {
        assert_eq!(error, Some(&BoardStoreError::Moved(moved())), "{result:?}");
    };
    let read = bob.read();
    is_moved(&read, read.as_ref().err());
    let made = commits(&repos.bob);
    // From the fence's head, and from the head before it, which a client
    // read before the cut-over.
    for from in [BoardHead(fence.clone()), before] {
        let write = bob.write_tasks(&from, &[task("b")], "board: add b");
        is_moved(&write, write.as_ref().err());
        let clear = bob.clear_set_aside(&from, &clear_of_a(), "board: clear");
        is_moved(&clear, clear.as_ref().err());
    }
    let bootstrap = bob.bootstrap();
    is_moved(&bootstrap, bootstrap.as_ref().err());
    assert_eq!(commits(&repos.bob), made, "no write made a commit");
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), fence);
    // The refusal names every URL.
    let told = told(&read.unwrap_err()).into_owned();
    assert!(
        told.contains("https://board.example:7443, http://100.64.0.2:7443"),
        "{told}"
    );
}

/// A `MOVED` this quecto would not write, or a fence's `format` tree that
/// holds anything but the marker and `MOVED`, is refused as malformed on
/// every read and write; the board is never written.
#[test]
fn a_malformed_moved_is_refused_and_never_written() {
    let fence = |moved: &[u8]| {
        vec![
            ("100644", "MOVED", Object::Blob(moved.to_vec())),
            ("100644", "board", Object::Blob(MOVED_FORMAT.to_vec())),
        ]
    };
    let mut huge = MOVED.to_vec();
    huge.resize(MAX_MOVED_BYTES + 1, b'\n');
    let cases: Vec<(&str, Vec<(&'static str, &'static str, Object)>)> = vec![
        ("an unknown key", fence(&[MOVED, b"mirror x\n"].concat())),
        (
            "an ftp url",
            fence(b"board_id 0123456789abcdef\nurl ftp://a.example\n"),
        ),
        ("no url", fence(b"board_id 0123456789abcdef\n")),
        (
            "a malformed id",
            fence(b"board_id ../etc\nurl https://a.example\n"),
        ),
        ("too many bytes", fence(&huge)),
        ("no MOVED", board_file(MOVED_FORMAT)),
        (
            "an executable MOVED",
            vec![
                ("100755", "MOVED", Object::Blob(MOVED.to_vec())),
                ("100644", "board", Object::Blob(MOVED_FORMAT.to_vec())),
            ],
        ),
        (
            "a tree for MOVED",
            vec![
                ("40000", "MOVED", Object::EmptyTree),
                ("100644", "board", Object::Blob(MOVED_FORMAT.to_vec())),
            ],
        ),
        (
            "a third entry",
            fence(MOVED)
                .into_iter()
                .chain([("100644", "more", Object::Blob(b"x".to_vec()))])
                .collect(),
        ),
        (
            "more after the marker",
            vec![
                ("100644", "MOVED", Object::Blob(MOVED.to_vec())),
                (
                    "100644",
                    "board",
                    Object::Blob(b"quecto-board 4\nmore\n".to_vec()),
                ),
            ],
        ),
    ];
    for (case, entries) in cases {
        let repos = Repos::new();
        created(&store(&repos.alice));
        let head = board_with(&repos, true, &[marker(entries)]);
        let bob = store(&repos.bob);
        let read = bob.read();
        assert!(
            matches!(&read, Err(BoardStoreError::MovedMalformed(_))),
            "{case}: {read:?}"
        );
        let write = bob.write_tasks(&head, &[task("b")], "board: add b");
        assert!(
            matches!(&write, Err(BoardStoreError::MovedMalformed(_))),
            "{case}: {write:?}"
        );
        assert_eq!(
            sh(&repos.remote(), &["rev-parse", BOARD_REF]),
            head.0,
            "{case}"
        );
    }
}

/// Fencing builds the fence's tree (the board's root, its format tree
/// replaced, strays dropped) and makes no commit; fencing the fence to the
/// same place is a no-op, and to another place is refused naming where
/// the board moved.
#[test]
fn fencing_builds_the_moved_tree_and_is_idempotent() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let stray = Root::File("100644", "notes.txt", b"x".to_vec());
    let head = board_with(&repos, true, &[marker(board_file(FORMAT)), stray]);
    let bob = store(&repos.bob);
    let made = commits(&repos.bob);
    let Ok(Fence::Tree(tree)) = bob.fence_tree(&head.0, &moved()) else {
        panic!("no fence tree")
    };
    assert_eq!(commits(&repos.bob), made, "fencing makes no commit");
    assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), head.0);
    assert_eq!(
        sh(&repos.bob, &["ls-tree", "-r", "--name-only", &tree]),
        "README.md\nformat/MOVED\nformat/board\ntasks/a.md"
    );
    let file = |path: &str| sh(&repos.bob, &["cat-file", "blob", &format!("{tree}:{path}")]);
    assert_eq!(file("format/board") + "\n", "quecto-board 4\n");
    assert_eq!(
        file("format/MOVED") + "\n",
        String::from_utf8(moved().file()).unwrap()
    );
    assert_eq!(moved().file(), MOVED);
    // The same commit fences to the same tree.
    assert_eq!(
        bob.fence_tree(&head.0, &moved()),
        Ok(Fence::Tree(tree.clone()))
    );
    // Once fenced: the same place is a no-op; another is refused.
    let fenced = sh(
        &repos.bob,
        &["commit-tree", &tree, "-p", &head.0, "-m", "board: fence"],
    );
    assert_eq!(bob.fence_tree(&fenced, &moved()), Ok(Fence::Already));
    let elsewhere = bob.fence_tree(&fenced, &moved_to(&["https://other.example"]));
    assert_eq!(elsewhere, Err(BoardStoreError::Moved(moved())));
    assert_eq!(
        bob.board_at(&fenced).err(),
        Some(BoardStoreError::Moved(moved()))
    );
}

/// A board this quecto does not read is never fenced.
#[test]
fn a_board_this_quecto_does_not_read_is_never_fenced() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let head = board_with(&repos, true, &[marker(board_file(b"quecto-board 1\n"))]);
    let fenced = store(&repos.bob).fence_tree(&head.0, &moved());
    assert!(
        matches!(fenced, Err(BoardStoreError::Unavailable(_))),
        "{fenced:?}"
    );
}
