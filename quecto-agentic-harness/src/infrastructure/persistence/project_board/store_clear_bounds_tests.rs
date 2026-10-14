//! The bounds a clear keeps: those every write keeps, a board at its
//! entry bound, and how much a clear may name and a refusal may list.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::{
    MAX_BOARD_ENTRIES, MAX_READ_ENTRIES,
};

/// A clear keeps every bound a write keeps, and is refused as invalid
/// past one, never panicking: the bytes of a tree and the task-file bytes
/// a read takes. (A clear never adds an entry, so a board a read takes
/// keeps within the entry bound once cleared.)
#[test]
fn a_clear_keeps_every_bound_a_write_keeps() {
    // The bytes of a tree, and the task-file bytes a read takes.
    let (repos, mut bob, head) = board_of(&[
        ("100644", "a.md", task_blob(&task("a"))),
        ("100644", "b.md", task_blob(&task("b"))),
        ("100644", "broken.md", Object::Blob(b"{".to_vec())),
        ("100644", "dup.md", task_blob(&task("dup"))),
        ("100644", "dup.md", task_blob(&other("dup"))),
    ]);
    let dup = oid_of(&repos.bob, &encode(&task("dup")).unwrap());
    let dup2 = oid_of(&repos.bob, &encode(&other("dup")).unwrap());
    let broken = oid_of(&repos.bob, b"{");
    let size = |task: &Task| encode(task).unwrap().len();
    let keeping = clearing(
        "tasks/broken.md",
        &broken,
        &[("tasks/dup.md", keep(&dup, &[&dup, &dup2]))],
    );
    bob.limits.board_bytes = size(&task("a")) + size(&task("b")) + size(&task("dup")) - 1;
    let past = bob.clear_set_aside(&head, &keeping, "board: clear");
    assert!(
        matches!(&past, Err(BoardStoreError::WriteTooLarge(why)) if why.contains("bytes")),
        "the copy kept counts: {past:?}"
    );
    bob.limits.board_bytes += 1;
    // The tasks tree the clear makes: each entry its mode, a space, its
    // name, a NUL and a 20-byte object name.
    let cleared: usize = ["a.md", "b.md", "dup.md"]
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

/// A board at its entry bound (its tasks, one file each, and a name listed
/// twice) is cleared, and the copy kept counts as one entry: a write then
/// fills the board to its bound and no further.
#[test]
fn a_board_at_its_entry_bound_is_cleared_and_written_up_to_it() {
    let mut files: Vec<(String, Vec<u8>)> = (0..MAX_TASK_ENTRIES - 2)
        .map(|n| {
            let task = task(&format!("t{n}"));
            (format!("t{n}.md"), encode(&task).unwrap())
        })
        .collect();
    files.push(("dup.md".into(), encode(&task("dup")).unwrap()));
    files.push(("dup.md".into(), encode(&other("dup")).unwrap()));
    let (repos, bob, head) = mktree_board(&files);
    let one = oid_of(&repos.bob, &encode(&task("dup")).unwrap());
    let two = oid_of(&repos.bob, &encode(&other("dup")).unwrap());
    let path = "tasks/dup.md";
    let keeping = clearing(path, &one, &[(path, keep(&one, &[&one, &two]))]);
    let cleared = written(bob.clear_set_aside(&head, &keeping, "board: clear dup"));
    let full = written(bob.write_tasks(&cleared, &[task("last")], "board: add last"));
    let past = bob.write_tasks(&full, &[task("over")], "board: add over");
    assert!(
        matches!(&past, Err(BoardStoreError::InvalidWrite(why)) if why.contains("at most")),
        "{past:?}"
    );
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
                    format!("d{n}.md"),
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
        refused(bob.clear_set_aside(&head, &clearing("tasks/d0.md", &one, &[]), "board: clear"));
    assert!(why.contains("and 2 more"), "{why}");
    // A board a read takes may list about as many names twice as it holds
    // entries in all its folders, halved: one clear must name them all.
    let many = |count: usize| -> Vec<(String, KeepCopy)> {
        (0..count)
            .map(|n| (format!("tasks/n{n}.md"), drop_all(&[])))
            .collect()
    };
    let clear_of = |names: &[(String, KeepCopy)]| {
        let names: Vec<(&str, KeepCopy)> = names
            .iter()
            .map(|(path, keep)| (path.as_str(), keep.clone()))
            .collect();
        bob.clear_set_aside(
            &head,
            &clearing("tasks/d0.md", &one, &names),
            "board: clear",
        )
    };
    let judged = clear_of(&many(MAX_READ_ENTRIES / 2 + 1));
    assert!(
        matches!(judged, Ok(BoardWrite::Refused(_))),
        "past one folder's share: {judged:?}"
    );
    let judged = clear_of(&many(MAX_BOARD_ENTRIES / 2));
    assert!(
        matches!(judged, Ok(BoardWrite::Refused(_))),
        "at the bound: {judged:?}"
    );
    let invalid = clear_of(&many(MAX_BOARD_ENTRIES / 2 + 1));
    assert!(
        matches!(invalid, Err(BoardStoreError::InvalidWrite(_))),
        "{invalid:?}"
    );
}

/// The copies a clear names are bounded by what it saw, the copy it
/// keeps not counted twice: at the bound it is judged, past it invalid.
#[test]
fn the_copies_a_clear_names_are_bounded_by_what_it_saw() {
    let (repos, bob, head) = board_of(&[
        ("100644", "dup.md", Object::Blob(b"one".to_vec())),
        ("100644", "dup.md", Object::Blob(b"two".to_vec())),
    ]);
    let one = oid_of(&repos.bob, b"one");
    let path = "tasks/dup.md";
    let objects = |count: usize| -> Vec<String> {
        std::iter::once(one.clone())
            .chain((1..count).map(|n| format!("{n:040x}")))
            .collect()
    };
    let at = objects(MAX_BOARD_ENTRIES);
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
    let past = objects(MAX_BOARD_ENTRIES + 1);
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

/// A board whose `tasks/` tree holds exactly `files` (name, bytes; a name
/// may repeat), made with `mktree`, on the remote; bob's store of it. Its
/// root is a legacy one, without the format marker.
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

/// A copy kept whose name is no task id is never read as a task, so it
/// never counts toward the task-file bytes a read takes.
#[test]
fn a_copy_kept_under_a_name_that_is_no_id_counts_no_read_bytes() {
    let (repos, mut bob, head) = board_of(&[
        ("100644", "a.md", task_blob(&task("a"))),
        ("100644", "Upper.md", Object::Blob(b"one".to_vec())),
        ("100644", "Upper.md", Object::Blob(b"two".to_vec())),
    ]);
    let [one, two] = [&b"one"[..], b"two"].map(|bytes| oid_of(&repos.bob, bytes));
    bob.limits.board_bytes = encode(&task("a")).unwrap().len();
    let path = "tasks/Upper.md";
    written(bob.clear_set_aside(
        &head,
        &clearing(path, &one, &[(path, keep(&two, &[&one, &two]))]),
        "board: clear Upper",
    ));
}
