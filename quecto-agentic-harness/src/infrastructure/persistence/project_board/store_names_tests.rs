//! Names and modes in a board tree a write could not carry as they are:
//! gitlinks, names quecto does not keep, copies of one name a write drops,
//! and root names a clash quotes.
use super::*;
use crate::domain::project_board::value_objects::slug::MAX_SLUG_BYTES;

/// A gitlink under `tasks/` is a stray, whatever object it names: a write
/// drops it, so one naming an object this clone holds as a blob (which
/// `mktree` refuses) never stops a write.
#[test]
fn a_gitlink_task_file_is_a_stray_and_never_stops_a_write() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let head = raw_board(
        &repos.remote(),
        &[
            (
                "100644",
                "ok.md",
                Object::Blob(encode(&task("ok")).unwrap()),
            ),
            ("160000", "mod.md", Object::Blob(b"held here".to_vec())),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let moved = written(bob.write_tasks(&head, &[task("fresh")], "board: add fresh"));
    assert_eq!(
        sh(
            &repos.remote(),
            &["ls-tree", "--name-only", &format!("{}:tasks", moved.0)]
        ),
        "fresh.md\nok.md"
    );
    let board = bob.board_at(&head.0).unwrap();
    assert_eq!(
        board
            .quarantined
            .iter()
            .map(|q| (q.path.as_str(), q.id().map(Slug::as_str)))
            .collect::<Vec<_>>(),
        [("tasks/mod.md", None)]
    );
}

/// Only task file names quecto can show and bound are carried forward: valid
/// UTF-8 of shown characters, no longer than an id's. Any other name is a
/// stray: dropped by the next write, never counted as a duplicate, and
/// quoted wherever it is shown.
#[test]
fn only_names_quecto_can_show_are_carried() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let longest = format!("{}.md", "A".repeat(MAX_SLUG_BYTES));
    // One byte past the longest name kept: an id of the most bytes and the
    // five bytes of `.json`, the longer task file extension.
    let too_long = format!("{}.md", "A".repeat(MAX_SLUG_BYTES + 3));
    let json = |id: &str| Object::Blob(encode(&task(id)).unwrap());
    let (ok, kept, long, odd, odder, shady) = (
        json("ok"),
        Object::Blob(b"kept".to_vec()),
        Object::Blob(b"long".to_vec()),
        Object::Blob(b"one".to_vec()),
        Object::Blob(b"two".to_vec()),
        Object::Blob(b"shady".to_vec()),
    );
    let head = raw_board_named(
        &repos.remote(),
        &[
            ("100644", longest.as_bytes(), &kept),
            ("100644", too_long.as_bytes(), &long),
            ("100644", "Bad\u{1b}[2J\u{202e}.md".as_bytes(), &shady),
            ("100644", b"ok.md", &ok),
            ("100644", b"\xfe.md", &odd),
            ("100644", b"\xff.md", &odder),
            ("100644", b"a/b.md", &odder),
            ("100644", b"a\\b.md", &odder),
            ("100644", b"..md", &odder),
            ("100644", b"a[x](y).md", &odder),
            ("100644", "\u{3164}.md".as_bytes(), &odder),
            ("100644", "e\u{301}.md".as_bytes(), &odder),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let board = bob.board_at(&head.0).unwrap();
    assert_eq!(
        board.tasks.keys().map(Slug::as_str).collect::<Vec<_>>(),
        ["ok"]
    );
    assert!(board.repeated.is_empty(), "{:?}", board.repeated);
    for file in &board.quarantined {
        assert!(!file.path.chars().any(char::is_control), "{:?}", file.path);
        assert!(!file.path.contains('\u{202e}'), "{:?}", file.path);
    }
    let moved = written(bob.write_tasks(&head, &[task("fresh")], "board: add fresh"));
    let names = sh(
        &repos.remote(),
        &[
            "ls-tree",
            "--name-only",
            "-z",
            &format!("{}:tasks", moved.0),
        ],
    );
    let expected = [longest.as_str(), "fresh.md", "ok.md"];
    assert_eq!(
        names
            .split('\0')
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>(),
        expected
    );
}

/// A name twice, one copy of which a write drops (a gitlink), is no
/// duplicate: the copy a write carries is read, and writes go on.
#[test]
fn a_name_twice_where_a_write_drops_one_copy_is_no_duplicate() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let head = raw_board(
        &repos.remote(),
        &[
            (
                "100644",
                "dup.md",
                Object::Blob(encode(&task("dup")).unwrap()),
            ),
            ("160000", "dup.md", Object::Blob(b"a gitlink".to_vec())),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let board = bob.board_at(&head.0).unwrap();
    assert!(board.repeated.is_empty(), "{:?}", board.repeated);
    assert_eq!(
        board.tasks.keys().map(Slug::as_str).collect::<Vec<_>>(),
        ["dup"]
    );
    let moved = written(bob.write_tasks(&head, &[task("fresh")], "board: add fresh"));
    assert_eq!(
        sh(&repos.remote(), &["ls-tree", &format!("{}:tasks", moved.0)])
            .lines()
            .filter(|row| row.ends_with("\tdup.md"))
            .count(),
        1
    );
}

/// A name at the root of a branch that is not a board is quoted in the
/// clash that names it.
#[test]
fn a_clash_quotes_the_root_name_it_names() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let mut heads = vec![];
    for repo in [&repos.remote(), &repos.bob] {
        let root = raw_tree(
            repo,
            &[
                ("100644", "README.md", Object::Blob(README.to_vec())),
                ("40000", "evil\u{1b}[2J\u{202e}", Object::EmptyTree),
            ],
        );
        heads.push(raw_commit(repo, &root));
    }
    let read = store(&repos.bob).board_at(&heads[1].0);
    let Err(BoardStoreError::Clash(why)) = &read else {
        panic!("{read:?}");
    };
    assert!(why.contains("evil"), "{why}");
    assert!(
        !why.chars().any(|c| c.is_control() || c == '\u{202e}'),
        "{why:?}"
    );
}

/// A name twice where one copy's mode and object disagree (a write drops
/// it): the copy a write carries is read, the other is set aside with no
/// id, so no task is both read and set aside, and writes go on.
#[test]
fn a_name_twice_where_one_copy_disagrees_with_its_mode_is_read_once() {
    for (mode, object) in [
        (
            "40000",
            Object::Blob(b"a blob under a directory mode".to_vec()),
        ),
        ("100644", Object::EmptyTree),
    ] {
        let repos = Repos::new();
        let alice = store(&repos.alice);
        created(&alice);
        let head = raw_board(
            &repos.remote(),
            &[
                (
                    "100644",
                    "dup.md",
                    Object::Blob(encode(&task("dup")).unwrap()),
                ),
                (mode, "dup.md", object),
            ],
            &[&repos.bob],
        );
        let bob = store(&repos.bob);
        let board = bob.board_at(&head.0).unwrap();
        assert_eq!(
            board.tasks.keys().map(Slug::as_str).collect::<Vec<_>>(),
            ["dup"],
            "{mode}"
        );
        assert!(
            board.quarantined.iter().all(|file| file.id().is_none()),
            "{mode}: {:?}",
            board.quarantined
        );
        written(bob.write_tasks(&head, &[task("fresh")], "board: add fresh"));
    }
}

/// The longest name kept is an id of the most bytes and `.json` (69
/// bytes): a legacy task file of the longest id is set aside under it and
/// carried, so the id stays blocked; one byte longer is a stray a write
/// drops.
#[test]
fn a_legacy_name_of_the_longest_id_is_kept_and_one_byte_more_dropped() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let longest = format!("{}.json", "a".repeat(MAX_SLUG_BYTES));
    let past = format!("{}.json", "b".repeat(MAX_SLUG_BYTES + 1));
    assert_eq!((longest.len(), past.len()), (69, 70));
    let legacy = Object::Blob(b"{}".to_vec());
    let head = raw_board_named(
        &repos.remote(),
        &[
            ("100644", longest.as_bytes(), &legacy),
            ("100644", past.as_bytes(), &legacy),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let board = bob.board_at(&head.0).unwrap();
    let id = "a".repeat(MAX_SLUG_BYTES);
    let found: Vec<(String, Option<&str>)> = board
        .quarantined
        .iter()
        .map(|file| (file.path.clone(), file.id().map(Slug::as_str)))
        .collect();
    assert_eq!(
        found,
        [
            (format!("tasks/{longest}"), Some(id.as_str())),
            (format!("tasks/{past}"), None),
        ]
    );
    let moved = written(bob.write_tasks(&head, &[task("fresh")], "board: add fresh"));
    assert_eq!(
        sh(
            &repos.remote(),
            &["ls-tree", "--name-only", &format!("{}:tasks", moved.0)]
        ),
        format!("{longest}\nfresh.md")
    );
}
