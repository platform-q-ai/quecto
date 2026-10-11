//! Clearing files set aside, over a real remote: which file a clear means
//! (path and object), every name listed more than once resolved in the
//! same clear, the copy kept as it was, the bounds every write keeps, and
//! a stale head answered first.
use super::*;
use crate::application::project_board::ports::{KeepCopy, SetAsideClear};

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

fn clearing(path: &str, object: Option<&str>, repeated: &[(&str, KeepCopy)]) -> SetAsideClear {
    SetAsideClear {
        path: path.into(),
        object: object.map(str::to_string),
        repeated: repeated
            .iter()
            .map(|(path, keep)| ((*path).to_string(), keep.clone()))
            .collect(),
    }
}

fn keep(oid: &str) -> KeepCopy {
    KeepCopy::Object(oid.to_string())
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
    let path = "tasks/dup.json";
    let why = refused(bob.clear_set_aside(&head, &clearing(path, None, &[]), "board: clear"));
    assert!(
        why.contains(&first) && why.contains(&second),
        "both copies named: {why}"
    );
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing(
            path,
            None,
            &[(
                path,
                keep(&oid_of(&repos.bob, &encode(&task("ok")).unwrap())),
            )],
        ),
        "board: clear",
    ));
    assert!(why.contains("not one of"), "{why}");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing(path, None, &[(path, keep(&second))]),
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
    let dup = oid_of(&repos.bob, &encode(&task("dup")).unwrap());
    let two = oid_of(&repos.bob, &encode(&other("two")).unwrap());
    for partial in [
        clearing("tasks/dup.json", None, &[("tasks/dup.json", keep(&dup))]),
        clearing("tasks/broken.json", None, &[("tasks/dup.json", keep(&dup))]),
    ] {
        let why = refused(bob.clear_set_aside(&head, &partial, "board: clear"));
        assert!(why.contains("tasks/two.json"), "{why}");
    }
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/broken.json",
            None,
            &[
                ("tasks/dup.json", keep(&dup)),
                ("tasks/two.json", KeepCopy::DropAll),
                ("tasks/ok.json", KeepCopy::DropAll),
            ],
        ),
        "board: clear",
    ));
    assert!(why.contains("tasks/ok.json"), "not listed twice: {why}");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/broken.json",
            None,
            &[
                ("tasks/dup.json", keep(&dup)),
                ("tasks/two.json", keep(&two)),
            ],
        ),
        "board: clear broken",
    ));
    let names: Vec<String> = listing(&repos.remote(), &cleared)
        .iter()
        .map(|row| row.split_once('\t').unwrap().1.to_string())
        .collect();
    assert_eq!(names, ["dup.json", "dup.md", "two.json", "two.md"]);
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
    let run = oid_of(&repos.bob, b"two");
    let tree = sh_with(&repos.bob, &["mktree"], b"");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/run.json",
            None,
            &[
                ("tasks/run.json", keep(&run)),
                ("tasks/dir.json", keep(&tree)),
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
            None,
            &[("tasks/dir.json", KeepCopy::DropAll)],
        ),
        "board: clear dir",
    ));
    assert_eq!(bob.read().unwrap().head, dropped);
    assert_eq!(bob.read().unwrap().quarantined, vec![]);
}

/// Two files that show one path are told apart by their objects: a clear
/// of one never touches the other, and a clear that names neither is
/// refused.
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
    let why =
        refused(bob.clear_set_aside(&head, &clearing("tasks/x.json", None, &[]), "board: clear"));
    assert!(why.contains(&carried) && why.contains(&stray), "{why}");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing("tasks/x.json", Some(&stray), &[]),
        "board: clear the stray",
    ));
    assert_eq!(
        listing(&repos.remote(), &cleared),
        [format!("100644 blob {carried}\tx.json")],
        "the carried file stays"
    );
}

/// A clear keeps every bound a write keeps, and is refused as invalid
/// past one, never panicking: the entries a tree may hold, the bytes of a
/// tree, and the task-file bytes a read takes.
#[test]
fn a_clear_keeps_every_bound_a_write_keeps() {
    // Task files with no summaries: a clear would add one for each.
    let repos = Repos::new();
    created(&store(&repos.alice));
    let remote = repos.remote();
    let files = repos.dir.path().join("files");
    std::fs::create_dir(&files).unwrap();
    let names: Vec<String> = (0..MAX_TASKS + 52).map(|n| format!("t{n}.json")).collect();
    for name in &names {
        let id = name.trim_end_matches(".json");
        std::fs::write(files.join(name), encode(&task(id)).unwrap()).unwrap();
    }
    std::fs::write(files.join("notes.txt"), b"stray").unwrap();
    let mut all = names.clone();
    all.push("notes.txt".into());
    let paths: String = all
        .iter()
        .map(|name| format!("{}\n", files.join(name).display()))
        .collect();
    let oids = sh_with(
        &remote,
        &["hash-object", "-w", "--stdin-paths"],
        paths.as_bytes(),
    );
    let entries: String = all
        .iter()
        .zip(oids.lines())
        .map(|(name, oid)| format!("100644 blob {oid}\t{name}\n"))
        .collect();
    let tasks = sh_with(&remote, &["mktree"], entries.as_bytes());
    let head = sh(&remote, &["rev-parse", BOARD_REF]);
    let readme = sh(&remote, &["rev-parse", &format!("{head}:README.md")]);
    let root = sh_with(
        &remote,
        &["mktree"],
        format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n").as_bytes(),
    );
    let commit = sh(&remote, &["commit-tree", &root, "-p", &head, "-m", "many"]);
    sh(&remote, &["update-ref", BOARD_REF, &commit]);
    let bob = store(&repos.bob);
    let board = bob.read().unwrap();
    let past = bob.clear_set_aside(
        &board.head,
        &clearing("tasks/notes.txt", None, &[]),
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
    let size = |task: &Task| encode(task).unwrap().len();
    let keeping = clearing("tasks/broken.json", None, &[("tasks/dup.json", keep(&dup))]);
    bob.limits.board_bytes = size(&task("a")) + size(&task("b")) + size(&task("dup")) - 1;
    let past = bob.clear_set_aside(&head, &keeping, "board: clear");
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("bytes")),
        "the copy kept counts: {past:?}"
    );
    bob.limits.board_bytes += 1;
    // The tasks tree the clear makes: each entry its mode, a space, its
    // name, a NUL and a 20-byte object name.
    let cleared: usize = ["a.json", "a.md", "b.json", "b.md", "dup.json", "dup.md"]
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

/// What a clear echoes is quoted, and an object it is given is checked to
/// be an object id before anything else.
#[test]
fn a_clear_quotes_what_it_echoes_and_checks_objects_first() {
    let (_repos, bob, head) = board_of(&[
        ("100644", "dup.json", json(&task("dup"))),
        ("100644", "dup.json", json(&other("dup"))),
    ]);
    for object in ["nope\u{1b}[2J", "HEAD", &"0".repeat(39)] {
        for clear in [
            clearing("tasks/dup.json", None, &[("tasks/dup.json", keep(object))]),
            clearing(
                "tasks/dup.json",
                Some(object),
                &[("tasks/dup.json", KeepCopy::DropAll)],
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
        &clearing(
            "tasks/\u{1b}[2Jgone.json\u{202e}",
            None,
            &[("tasks/dup.json", KeepCopy::DropAll)],
        ),
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
    let dup = oid_of(&repos.alice, &encode(&task("dup")).unwrap());
    // Its own message, so its commit is never the one bob would make.
    written(alice.clear_set_aside(
        &moved,
        &clearing("tasks/broken.json", None, &[("tasks/dup.json", keep(&dup))]),
        "board: alice clears broken",
    ));
    let zeros = "0".repeat(dup.len());
    for clear in [
        clearing("tasks/broken.json", None, &[("tasks/dup.json", keep(&dup))]),
        clearing("tasks/ok.json", None, &[("tasks/dup.json", keep(&dup))]),
        clearing("tasks/dup.json", None, &[("tasks/dup.json", keep(&zeros))]),
        clearing("tasks/dup.json", None, &[]),
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
    let why = refused(bob.clear_set_aside(
        &head,
        &clearing(
            "tasks/broken.json",
            None,
            &[("tasks/broken.json", KeepCopy::DropAll)],
        ),
        "board: clear broken",
    ));
    assert!(why.contains("tasks/broken.json"), "{why}");
    let cleared = written(bob.clear_set_aside(
        &head,
        &clearing("tasks/broken.json", None, &[]),
        "board: clear broken",
    ));
    assert_eq!(
        listing(&repos.remote(), &cleared).len(),
        2,
        "ok.json and its summary"
    );
    written(bob.write(&cleared, &[task("broken")], "board: write broken"));
}
