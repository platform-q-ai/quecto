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

/// A file whose name looks like a secret is listed by its name, never
/// redacted (two such names stay two paths), and a clear by that name
/// removes it: a read afterwards finds neither.
#[test]
fn a_file_whose_name_looks_like_a_secret_is_cleared_by_its_name() {
    let (repos, bob, head) = board_of(&[
        ("100644", "token=x.json", Object::Blob(b"{".to_vec())),
        ("100644", "sk-abcdefgh.json", Object::Blob(b"[".to_vec())),
        ("100644", "ok.json", json(&task("ok"))),
    ]);
    let board = bob.read().unwrap();
    let mut paths: Vec<&str> = board
        .quarantined
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, ["tasks/sk-abcdefgh.json", "tasks/token=x.json"]);
    let mut at = head;
    for (path, bytes) in [
        ("tasks/token=x.json", &b"{"[..]),
        ("tasks/sk-abcdefgh.json", b"["),
    ] {
        let object = oid_of(&repos.bob, bytes);
        at = written(bob.clear_set_aside(&at, &clearing(path, &object, &[]), "board: clear"));
    }
    let after = bob.read().unwrap();
    assert_eq!(after.head, at);
    assert_eq!(after.quarantined, vec![], "both are gone");
    let names = listing(&repos.remote(), &at);
    assert!(
        names
            .iter()
            .all(|name| !name.contains("token=") && !name.contains("sk-")),
        "{names:?}"
    );
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

#[path = "store_clear_bounds_tests.rs"]
mod bounds;
