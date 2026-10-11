//! Clearing files set aside, over a real remote: which file a clear means
//! (path and object), every name listed more than once resolved in the
//! same clear, the copy kept as it was, the bounds every write keeps, and
//! a stale head answered first.
use super::*;
use crate::application::project_board::ports::{KeepCopy, SetAsideClear};
use std::collections::BTreeSet;

/// The entries of the board's `tasks/` tree at `head` on `repo`, as
/// `mode type oid\tname` rows in tree order.
fn listing(repo: &Path, head: &BoardHead) -> Vec<String> {
    sh(repo, &["ls-tree", &format!("{}:tasks", head.0)])
        .lines()
        .map(str::to_string)
        .collect()
}

fn other(id: &str) -> Task {
    let mut fields = task(id).into_fields();
    fields.title = "Another".into();
    Task::new(fields).unwrap()
}

fn oid_of(repo: &Path, bytes: &[u8]) -> String {
    sh_with(repo, &["hash-object", "--stdin"], bytes)
}

fn clearing(path: &str, object: &str, repeated: &[(&str, KeepCopy)]) -> SetAsideClear {
    SetAsideClear {
        path: path.into(),
        object: object.into(),
        repeated: repeated
            .iter()
            .map(|(path, keep)| ((*path).to_string(), keep.clone()))
            .collect(),
    }
}

fn seen(objects: &[&str]) -> BTreeSet<String> {
    objects.iter().map(|object| (*object).to_string()).collect()
}

/// Keep `object`, having seen the copies `objects`.
fn keep(object: &str, objects: &[&str]) -> KeepCopy {
    KeepCopy::Object {
        keep: object.to_string(),
        seen: seen(objects),
    }
}

fn drop_all(objects: &[&str]) -> KeepCopy {
    KeepCopy::Drop(seen(objects))
}

/// The reason a clear was refused, or a panic naming what it was instead.
fn refused(outcome: Result<BoardWrite, BoardStoreError>) -> String {
    match outcome {
        Ok(BoardWrite::Refused(why)) => why,
        other => panic!("not refused: {other:?}"),
    }
}

/// A board of `entries` on the remote and `bob`, and bob's store of it.
fn board_of(entries: &[(&str, &str, Object)]) -> (Repos, GitBoardStore, BoardHead) {
    let repos = Repos::new();
    created(&store(&repos.alice));
    raw_board(&repos.remote(), entries, &[&repos.bob]);
    let bob = store(&repos.bob);
    let head = bob.read().unwrap().head;
    (repos, bob, head)
}

fn json(task: &Task) -> Object {
    Object::Blob(encode(task).unwrap())
}

#[test]
fn a_name_listed_twice_is_cleared_by_naming_the_copy_to_keep() {
    let (repos, bob, head) = board_of(&[
        ("100644", "dup.json", json(&task("dup"))),
        ("100644", "dup.json", json(&other("dup"))),
        ("100644", "ok.json", json(&task("ok"))),
    ]);
    let first = oid_of(&repos.bob, &encode(&task("dup")).unwrap());
    let second = oid_of(&repos.bob, &encode(&other("dup")).unwrap());
    let both = [first.as_str(), second.as_str()];
    let path = "tasks/dup.json";
    let why = refused(bob.clear_set_aside(&head, &clearing(path, &first, &[]), "board: clear"));
    assert!(
        why.contains(&first) && why.contains(&second),
        "both copies named: {why}"
    );
    let ok = oid_of(&repos.bob, &encode(&task("ok")).unwrap());
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing(path, &first, &[(path, keep(&ok, &[&first, &second, &ok]))]),
        "board: clear",
    ));
    assert!(why.contains("not one of") || why.contains("saw"), "{why}");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing(path, &second, &[(path, keep(&second, &both))]),
        "board: clear dup",
    ));
    let after = bob.read().unwrap();
    assert_eq!(after.quarantined, vec![]);
    assert!(
        after.tasks.contains(&other("dup")),
        "the copy kept is the task now"
    );
    let dup: Vec<String> = listing(&repos.remote(), &cleared)
        .into_iter()
        .filter(|row| row.ends_with("\tdup.json"))
        .collect();
    assert_eq!(dup, [format!("100644 blob {second}\tdup.json")]);
    assert_eq!(bob.committed_by(&cleared.0).unwrap(), bob.writer());
    assert_eq!(
        sh(&repos.remote(), &["log", "-1", "--format=%s", &cleared.0]),
        "board: clear dup"
    );
}

/// A copy the caller never saw is never dropped: a name whose copies are
/// not exactly the ones it names is refused, keeping one or none.
#[test]
fn a_copy_the_caller_never_saw_is_never_dropped() {
    let (repos, bob, head) = board_of(&[
        ("100644", "dup.json", Object::Blob(b"one".to_vec())),
        ("100644", "dup.json", Object::Blob(b"two".to_vec())),
        ("100644", "dup.json", Object::Blob(b"three".to_vec())),
    ]);
    let [one, two, three] = [&b"one"[..], b"two", b"three"].map(|bytes| oid_of(&repos.bob, bytes));
    let path = "tasks/dup.json";
    let gone = oid_of(&repos.bob, b"gone");
    for resolution in [
        drop_all(&[&one, &two]),
        keep(&one, &[&one, &two]),
        drop_all(&[&one, &two, &three, &gone]),
    ] {
        let why = refused(bob.clear_set_aside(
            &head,
            &clearing(path, &one, &[(path, resolution.clone())]),
            "board: clear",
        ));
        assert!(why.contains(path), "{resolution:?}: {why}");
    }
    written(bob.clear_set_aside(
        &head,
        &clearing(path, &one, &[(path, drop_all(&[&one, &two, &three]))]),
        "board: clear dup",
    ));
}

/// While names are listed more than once, one clear resolves them all, so
/// it never pushes a tree a remote that checks its objects refuses. A
/// clear of another file names them too.
#[test]
fn every_name_listed_twice_is_resolved_in_one_clear_a_checking_remote_takes() {
    let (repos, bob, head) = board_of(&[
        ("100644", "broken.json", Object::Blob(b"{".to_vec())),
        ("100644", "dup.json", json(&task("dup"))),
        ("100644", "dup.json", json(&other("dup"))),
        ("100644", "two.json", json(&task("two"))),
        ("100644", "two.json", json(&other("two"))),
    ]);
    sh(&repos.remote(), &["config", "receive.fsckObjects", "true"]);
    let oid = |task: &Task| oid_of(&repos.bob, &encode(task).unwrap());
    let (dup, dup2, two, two2) = (
        oid(&task("dup")),
        oid(&other("dup")),
        oid(&task("two")),
        oid(&other("two")),
    );
    let broken = oid_of(&repos.bob, b"{");
    let dups = [dup.as_str(), dup2.as_str()];
    let twos = [two.as_str(), two2.as_str()];
    for partial in [
        clearing(
            "tasks/dup.json",
            &dup,
            &[("tasks/dup.json", keep(&dup, &dups))],
        ),
        clearing(
            "tasks/broken.json",
            &broken,
            &[("tasks/dup.json", keep(&dup, &dups))],
        ),
    ] {
        let why = refused(bob.clear_set_aside(&head, &partial, "board: clear"));
        assert!(why.contains("tasks/two.json"), "{why}");
    }
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/broken.json",
            &broken,
            &[
                ("tasks/dup.json", keep(&dup, &dups)),
                ("tasks/two.json", drop_all(&twos)),
                ("tasks/ok.json", drop_all(&[])),
            ],
        ),
        "board: clear",
    ));
    assert!(why.contains("tasks/ok.json"), "not listed twice: {why}");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/broken.json",
            &broken,
            &[
                ("tasks/dup.json", keep(&dup, &dups)),
                ("tasks/two.json", keep(&two2, &twos)),
            ],
        ),
        "board: clear broken",
    ));
    let names: Vec<String> = listing(&repos.remote(), &cleared)
        .iter()
        .map(|row| row.split_once('\t').unwrap().1.to_string())
        .collect();
    // The copies kept are carried as they are; the next write regenerates
    // their summaries.
    assert_eq!(names, ["dup.json", "two.json"]);
    let board = bob.read().unwrap();
    assert_eq!(board.quarantined, vec![]);
    written(bob.write(&board.head, &[task("fresh")], "board: add fresh"));
}

/// A copy kept stays as it was, mode and type; a name listed twice as
/// directories is cleared too, keeping one or dropping every copy.
#[test]
fn a_copy_kept_keeps_its_mode_and_directories_can_be_cleared() {
    let (repos, bob, head) = board_of(&[
        ("100755", "run.json", Object::Blob(b"one".to_vec())),
        ("100755", "run.json", Object::Blob(b"two".to_vec())),
        ("40000", "dir.json", Object::EmptyTree),
        ("40000", "dir.json", Object::EmptyTree),
    ]);
    let (run1, run) = (oid_of(&repos.bob, b"one"), oid_of(&repos.bob, b"two"));
    let tree = sh_with(&repos.bob, &["mktree"], b"");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/run.json",
            &run,
            &[
                ("tasks/run.json", keep(&run, &[&run1, &run])),
                ("tasks/dir.json", keep(&tree, &[&tree])),
            ],
        ),
        "board: clear run",
    ));
    assert_eq!(
        listing(&repos.remote(), &cleared),
        [
            format!("040000 tree {tree}\tdir.json"),
            format!("100755 blob {run}\trun.json")
        ]
    );
    let (_repos, bob, head) = board_of(&[
        ("40000", "dir.json", Object::EmptyTree),
        ("40000", "dir.json", Object::EmptyTree),
    ]);
    let dropped = written(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/dir.json",
            &tree,
            &[("tasks/dir.json", drop_all(&[&tree]))],
        ),
        "board: clear dir",
    ));
    assert_eq!(bob.read().unwrap().head, dropped);
    assert_eq!(bob.read().unwrap().quarantined, vec![]);
}

/// One object that names copies of different modes is no copy to keep:
/// which one stays would be a guess.
#[test]
fn an_object_naming_copies_of_different_modes_is_never_kept() {
    let (repos, bob, head) = board_of(&[
        ("100644", "x.json", Object::Blob(b"same".to_vec())),
        ("100755", "x.json", Object::Blob(b"same".to_vec())),
    ]);
    let same = oid_of(&repos.bob, b"same");
    let path = "tasks/x.json";
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing(path, &same, &[(path, keep(&same, &[&same]))]),
        "board: clear",
    ));
    assert!(why.contains("modes"), "{why}");
    written(bob.clear_set_aside(
        &head,
        &clearing(path, &same, &[(path, drop_all(&[&same]))]),
        "board: clear x",
    ));
}

/// Two files that show one path are told apart by their objects: a clear
/// of one never touches the other. Two that also share one object are one
/// file to clear.
#[test]
fn a_path_two_files_share_is_cleared_by_object_alone() {
    let (repos, bob, head) = board_of(&[
        (
            "100644",
            "x.json",
            Object::Blob(b"{\"schema\": 999}".to_vec()),
        ),
        ("100644", "x.json\u{200b}", Object::Blob(b"stray".to_vec())),
    ]);
    let (carried, stray) = (
        oid_of(&repos.bob, b"{\"schema\": 999}"),
        oid_of(&repos.bob, b"stray"),
    );
    let board = bob.read().unwrap();
    let shown: Vec<&str> = board
        .quarantined
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(shown, ["tasks/x.json", "tasks/x.json"]);
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing("tasks/x.json", &stray, &[]),
        "board: clear the stray",
    ));
    assert_eq!(
        listing(&repos.remote(), &cleared),
        [format!("100644 blob {carried}\tx.json")],
        "the carried file stays"
    );
    // Any write drops strays, so the twins are a board of their own.
    let (repos, bob, head) = board_of(&[
        ("100644", "y.json\u{200b}", Object::Blob(b"twin".to_vec())),
        ("100644", "y.json\u{200c}", Object::Blob(b"twin".to_vec())),
    ]);
    let twin = oid_of(&repos.bob, b"twin");
    written(bob.clear_set_aside(
        &head,
        &clearing("tasks/y.json", &twin, &[]),
        "board: clear y",
    ));
}

/// The object of a file to clear is one of its copies, also when its
/// path is a name listed more than once.
#[test]
fn the_object_of_a_name_listed_twice_is_one_of_its_copies() {
    let (repos, bob, head) = board_of(&[
        ("100644", "dup.json", Object::Blob(b"one".to_vec())),
        ("100644", "dup.json", Object::Blob(b"two".to_vec())),
    ]);
    let [one, two] = [&b"one"[..], b"two"].map(|bytes| oid_of(&repos.bob, bytes));
    let other = oid_of(&repos.bob, b"other");
    let path = "tasks/dup.json";
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing(path, &other, &[(path, drop_all(&[&one, &two]))]),
        "board: clear",
    ));
    assert!(why.contains(&other), "{why}");
}

/// A clear keeps every bound a write keeps, and is refused as invalid
/// past one, never panicking: the entries a tree may hold, the bytes of a
/// tree, and the task-file bytes a read takes.
#[test]
fn a_clear_keeps_every_bound_a_write_keeps() {
    // Task files with no summaries: a clear would add one for each.
    let (repos, bob, head) = mktree_board(
        &(0..MAX_TASKS + 52)
            .map(|n| {
                (
                    format!("t{n}.json"),
                    encode(&task(&format!("t{n}"))).unwrap(),
                )
            })
            .chain([("notes.txt".to_string(), b"stray".to_vec())])
            .collect::<Vec<_>>(),
    );
    let notes = oid_of(&repos.bob, b"stray");
    let past = bob.clear_set_aside(
        &head,
        &clearing("tasks/notes.txt", &notes, &[]),
        "board: clear",
    );
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("at most")),
        "{past:?}"
    );

    // The bytes of a tree, and the task-file bytes a read takes.
    let (repos, mut bob, head) = board_of(&[
        ("100644", "a.json", json(&task("a"))),
        ("100644", "b.json", json(&task("b"))),
        ("100644", "broken.json", Object::Blob(b"{".to_vec())),
        ("100644", "dup.json", json(&task("dup"))),
        ("100644", "dup.json", json(&other("dup"))),
    ]);
    let dup = oid_of(&repos.bob, &encode(&task("dup")).unwrap());
    let dup2 = oid_of(&repos.bob, &encode(&other("dup")).unwrap());
    let broken = oid_of(&repos.bob, b"{");
    let size = |task: &Task| encode(task).unwrap().len();
    let keeping = clearing(
        "tasks/broken.json",
        &broken,
        &[("tasks/dup.json", keep(&dup, &[&dup, &dup2]))],
    );
    bob.limits.board_bytes = size(&task("a")) + size(&task("b")) + size(&task("dup")) - 1;
    let past = bob.clear_set_aside(&head, &keeping, "board: clear");
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("bytes")),
        "the copy kept counts: {past:?}"
    );
    bob.limits.board_bytes += 1;
    // The tasks tree the clear makes: each entry its mode, a space, its
    // name, a NUL and a 20-byte object name.
    let cleared: usize = ["a.json", "a.md", "b.json", "b.md", "dup.json"]
        .iter()
        .map(|name| "100644".len() + 1 + name.len() + 1 + 20)
        .sum();
    bob.limits.tree_bytes = cleared - 1;
    let past = bob.clear_set_aside(&head, &keeping, "board: clear");
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("tree")),
        "{past:?}"
    );
    bob.limits.tree_bytes = LIMITS.tree_bytes;
    written(bob.clear_set_aside(&head, &keeping, "board: clear broken"));
}

/// A copy a clear keeps that a read will read as a task needs a summary
/// the next write adds, so it counts as two entries: at the bound the
/// clear is written, past it refused.
#[test]
fn a_copy_kept_counts_with_the_summary_it_will_need() {
    let tasks: Vec<(String, Vec<u8>)> = (0..MAX_TASKS - 1)
        .flat_map(|n| {
            let task = task(&format!("t{n}"));
            [
                (format!("t{n}.json"), encode(&task).unwrap()),
                (format!("t{n}.md"), summary(&task).into_bytes()),
            ]
        })
        .collect();
    let dup = |tasks: &[(String, Vec<u8>)], extra: &[(String, Vec<u8>)]| {
        let mut files = tasks.to_vec();
        files.push(("dup.json".into(), encode(&task("dup")).unwrap()));
        files.push(("dup.json".into(), encode(&other("dup")).unwrap()));
        files.extend(extra.iter().cloned());
        mktree_board(&files)
    };
    let (repos, bob, head) = dup(&tasks, &[]);
    let one = oid_of(&repos.bob, &encode(&task("dup")).unwrap());
    let two = oid_of(&repos.bob, &encode(&other("dup")).unwrap());
    let path = "tasks/dup.json";
    let keeping = clearing(path, &one, &[(path, keep(&one, &[&one, &two]))]);
    written(bob.clear_set_aside(&head, &keeping, "board: clear dup"));
    // One task without its summary yet and one file carried as it is: the
    // tree still holds 4,096 entries, and the copy kept takes the board
    // past them only when it counts with its summary.
    let without = format!("t{}.md", MAX_TASKS - 2);
    let fewer: Vec<(String, Vec<u8>)> = tasks
        .iter()
        .filter(|(name, _)| *name != without)
        .cloned()
        .collect();
    let (_repos, bob, head) = dup(&fewer, &[("odd.json".into(), b"{".to_vec())]);
    let past = bob.clear_set_aside(&head, &keeping, "board: clear dup");
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("at most")),
        "{past:?}"
    );
    written(bob.clear_set_aside(
        &head,
        &clearing(path, &one, &[(path, drop_all(&[&one, &two]))]),
        "board: drop dup",
    ));
}

/// What a clear may name is bounded, and a refusal lists a bounded number
/// of names, then how many more.
#[test]
fn a_clear_and_its_refusals_are_bounded() {
    let entries: Vec<(String, String, Object)> = (0..10)
        .flat_map(|n| {
            [b"one", b"two"].map(|copy| {
                let mut bytes = copy.to_vec();
                bytes.push(b'0' + n);
                (
                    "100644".to_string(),
                    format!("d{n}.json"),
                    Object::Blob(bytes),
                )
            })
        })
        .collect();
    let entries: Vec<(&str, &str, Object)> = entries
        .into_iter()
        .map(|(mode, name, object)| {
            (
                &*Box::leak(mode.into_boxed_str()),
                &*Box::leak(name.into_boxed_str()),
                object,
            )
        })
        .collect();
    let (repos, bob, head) = board_of(&entries);
    let one = oid_of(&repos.bob, b"one0");
    let why =
        refused(bob.clear_set_aside(&head, &clearing("tasks/d0.json", &one, &[]), "board: clear"));
    assert!(why.contains("and 2 more"), "{why}");
    let many: Vec<(String, KeepCopy)> = (0..=MAX_TASK_ENTRIES / 2)
        .map(|n| (format!("tasks/n{n}.json"), drop_all(&[])))
        .collect();
    let many: Vec<(&str, KeepCopy)> = many
        .iter()
        .map(|(path, keep)| (path.as_str(), keep.clone()))
        .collect();
    let invalid = bob.clear_set_aside(
        &head,
        &clearing("tasks/d0.json", &one, &many),
        "board: clear",
    );
    assert!(
        matches!(invalid, Err(BoardStoreError::InvalidWrite(_))),
        "{invalid:?}"
    );
}

/// What a clear echoes is quoted, and an object it is given is checked to
/// be an object id before anything else.
#[test]
fn a_clear_quotes_what_it_echoes_and_checks_objects_first() {
    let (repos, bob, head) = board_of(&[
        ("100644", "dup.json", json(&task("dup"))),
        ("100644", "dup.json", json(&other("dup"))),
    ]);
    let dup = oid_of(&repos.bob, &encode(&task("dup")).unwrap());
    for object in ["nope\u{1b}[2J", "HEAD", &"0".repeat(39), &"A".repeat(40)] {
        for clear in [
            clearing(
                "tasks/dup.json",
                &dup,
                &[("tasks/dup.json", keep(object, &[object]))],
            ),
            clearing(
                "tasks/dup.json",
                object,
                &[("tasks/dup.json", drop_all(&[&dup]))],
            ),
            clearing(
                "tasks/dup.json",
                &dup,
                &[("tasks/dup.json", drop_all(&[object]))],
            ),
        ] {
            let invalid = bob.clear_set_aside(&head, &clear, "board: clear");
            assert!(
                matches!(&invalid, Err(BoardStoreError::InvalidWrite(why)) if !why.chars().any(char::is_control)),
                "{object:?}: {invalid:?}"
            );
        }
    }
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing("tasks/\u{1b}[2Jgone.json\u{202e}", &dup, &[]),
        "board: clear",
    ));
    assert!(
        !why.chars().any(|c| c.is_control() || c == '\u{202e}'),
        "{why:?}"
    );
}

/// A clear from a head the board moved past is lost, whatever else is
/// wrong with it: the head is judged first.
#[test]
fn a_clear_from_a_stale_head_is_lost_before_anything_else_is_judged() {
    let (repos, bob, stale) = board_of(&[
        ("100644", "broken.json", Object::Blob(b"{".to_vec())),
        ("100644", "dup.json", json(&task("dup"))),
        ("100644", "dup.json", json(&other("dup"))),
        ("100644", "ok.json", json(&task("ok"))),
    ]);
    let alice = store(&repos.alice);
    let moved = alice.read().unwrap().head;
    let oid = |task: &Task| oid_of(&repos.alice, &encode(task).unwrap());
    let (dup, dup2) = (oid(&task("dup")), oid(&other("dup")));
    let dups = [dup.as_str(), dup2.as_str()];
    let broken = oid_of(&repos.alice, b"{");
    // Its own message, so its commit is never the one bob would make.
    written(alice.clear_set_aside(
        &moved,
        &clearing(
            "tasks/broken.json",
            &broken,
            &[("tasks/dup.json", keep(&dup, &dups))],
        ),
        "board: alice clears broken",
    ));
    let zeros = "0".repeat(dup.len());
    for clear in [
        clearing(
            "tasks/broken.json",
            &broken,
            &[("tasks/dup.json", keep(&dup, &dups))],
        ),
        clearing(
            "tasks/ok.json",
            &broken,
            &[("tasks/dup.json", keep(&dup, &dups))],
        ),
        clearing(
            "tasks/dup.json",
            &dup,
            &[("tasks/dup.json", keep(&zeros, &[&zeros]))],
        ),
        clearing("tasks/dup.json", &dup, &[]),
    ] {
        assert_eq!(
            bob.clear_set_aside(&stale, &clear, "board: clear"),
            Ok(BoardWrite::Lost),
            "{clear:?}"
        );
    }
}

#[test]
fn a_file_set_aside_once_is_cleared_without_a_copy_to_keep() {
    let (repos, bob, head) = board_of(&[
        ("100644", "broken.json", Object::Blob(b"{".to_vec())),
        ("100644", "ok.json", json(&task("ok"))),
    ]);
    let broken = oid_of(&repos.bob, b"{");
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/broken.json",
            &broken,
            &[("tasks/broken.json", drop_all(&[&broken]))],
        ),
        "board: clear broken",
    ));
    assert!(why.contains("tasks/broken.json"), "{why}");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing("tasks/broken.json", &broken, &[]),
        "board: clear broken",
    ));
    assert_eq!(
        listing(&repos.remote(), &cleared).len(),
        2,
        "ok.json and its summary"
    );
    written(bob.write(&cleared, &[task("broken")], "board: write broken"));
}

/// A board whose `tasks/` tree holds exactly `files` (name, bytes; a name
/// may repeat), made with `mktree`, on the remote; bob's store of it.
fn mktree_board(files: &[(String, Vec<u8>)]) -> (Repos, GitBoardStore, BoardHead) {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let remote = repos.remote();
    let dir = repos.dir.path().join("files");
    std::fs::create_dir(&dir).unwrap();
    let paths: String = files
        .iter()
        .enumerate()
        .map(|(n, (_, bytes))| {
            let path = dir.join(n.to_string());
            std::fs::write(&path, bytes).unwrap();
            format!("{}\n", path.display())
        })
        .collect();
    let oids = sh_with(
        &remote,
        &["hash-object", "-w", "--stdin-paths"],
        paths.as_bytes(),
    );
    let entries: String = files
        .iter()
        .zip(oids.lines())
        .map(|((name, _), oid)| format!("100644 blob {oid}\t{name}\n"))
        .collect();
    let tasks = sh_with(&remote, &["mktree"], entries.as_bytes());
    let tip = sh(&remote, &["rev-parse", BOARD_REF]);
    let readme = sh(&remote, &["rev-parse", &format!("{tip}:README.md")]);
    let root = sh_with(
        &remote,
        &["mktree"],
        format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n").as_bytes(),
    );
    let commit = sh(
        &remote,
        &["commit-tree", &root, "-p", &tip, "-m", "hand edit"],
    );
    sh(&remote, &["update-ref", BOARD_REF, &commit]);
    let bob = store(&repos.bob);
    let head = bob.read().unwrap().head;
    (repos, bob, head)
}

/// A clear names what the listing shows at a repeated path: copies a
/// write carries, a copy it drops (a mode over another type) and a twin
/// that shows the same path. Only a carried copy may be kept.
#[test]
fn a_repeated_path_is_cleared_as_the_listing_shows_it() {
    let (repos, bob, head) = board_of(&[
        ("100644", "dup.json", Object::Blob(b"one".to_vec())),
        ("100644", "dup.json", Object::Blob(b"two".to_vec())),
        ("40000", "dup.json", Object::Blob(b"not a tree".to_vec())),
        ("100644", "dup.json\u{200b}", Object::Blob(b"twin".to_vec())),
    ]);
    let [one, two, odd, twin] =
        [&b"one"[..], b"two", b"not a tree", b"twin"].map(|bytes| oid_of(&repos.bob, bytes));
    let path = "tasks/dup.json";
    let listed: Vec<String> = bob
        .read()
        .unwrap()
        .quarantined
        .iter()
        .filter(|file| file.path == path)
        .map(|file| file.object.clone())
        .collect();
    assert_eq!(listed.len(), 4, "{listed:?}");
    let all = [one.as_str(), two.as_str(), odd.as_str(), twin.as_str()];
    for kept in [&odd, &twin] {
        let why = refused(bob.clear_set_aside(
            &head,
            &clearing(path, &one, &[(path, keep(kept, &all))]),
            "board: clear",
        ));
        assert!(
            why.contains(kept.as_str()),
            "only a carried copy is kept: {why}"
        );
    }
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing(path, &odd, &[(path, keep(&two, &all))]),
        "board: clear dup",
    ));
    assert_eq!(
        listing(&repos.remote(), &cleared),
        [format!("100644 blob {two}\tdup.json")]
    );
}

/// The copies a clear names are bounded by what it saw, the copy it
/// keeps not counted twice: at the bound it is judged, past it invalid.
#[test]
fn the_copies_a_clear_names_are_bounded_by_what_it_saw() {
    let (repos, bob, head) = board_of(&[
        ("100644", "dup.json", Object::Blob(b"one".to_vec())),
        ("100644", "dup.json", Object::Blob(b"two".to_vec())),
    ]);
    let one = oid_of(&repos.bob, b"one");
    let path = "tasks/dup.json";
    let objects = |count: usize| -> Vec<String> {
        std::iter::once(one.clone())
            .chain((1..count).map(|n| format!("{n:040x}")))
            .collect()
    };
    let at = objects(MAX_TASK_ENTRIES);
    let at: Vec<&str> = at.iter().map(String::as_str).collect();
    let judged = bob.clear_set_aside(
        &head,
        &clearing(path, &one, &[(path, keep(&one, &at))]),
        "board: clear",
    );
    assert!(
        matches!(judged, Ok(BoardWrite::Refused(_))),
        "at the bound: {judged:?}"
    );
    let past = objects(MAX_TASK_ENTRIES + 1);
    let past: Vec<&str> = past.iter().map(String::as_str).collect();
    for resolution in [keep(&one, &past), drop_all(&past)] {
        let invalid = bob.clear_set_aside(
            &head,
            &clearing(path, &one, &[(path, resolution)]),
            "board: clear",
        );
        assert!(
            matches!(invalid, Err(BoardStoreError::InvalidWrite(_))),
            "{invalid:?}"
        );
    }
}
