//! The board's format marker (#2527 review): every write leaves
//! `format/board` reading `quecto-board 3` (2 until #2481 S9 read the other
//! folders), a root tree an older quecto takes for a clash, or for a newer
//! format. A board of format 2 is read as it is. A root without it is a board written before it, read
//! as it is; a newer format asks for an upgrade before anything else is
//! judged; a damaged marker beside a board asks for the board to be moved
//! back; a file named `format` at the root is a stray.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::FORMAT;

/// A tree of raw `rows` (mode, name, object id) in `repo`, written as git
/// would never write one.
pub(super) fn raw_rows(repo: &Path, rows: &[(&str, &str, String)]) -> String {
    let mut bytes = vec![];
    for (mode, name, oid) in rows {
        bytes.extend(format!("{mode} {name}\0").as_bytes());
        bytes.extend(
            (0..oid.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&oid[at..at + 2], 16).unwrap()),
        );
    }
    sh_with(
        repo,
        &["hash-object", "-t", "tree", "--literally", "-w", "--stdin"],
        &bytes,
    )
}

pub(super) fn blob_in(repo: &Path, bytes: &[u8]) -> String {
    sh_with(repo, &["hash-object", "-w", "--stdin"], bytes)
}

/// What a row of the board's root holds.
enum Root {
    /// A `format` tree of these rows (mode, name, bytes or a subtree).
    Format(Vec<(&'static str, &'static str, Object)>),
    /// A file of these bytes under this name and mode.
    File(&'static str, &'static str, Vec<u8>),
    /// A row of this mode over a blob of these bytes (a mode git never
    /// writes over one).
    ModeOverBlob(&'static str, &'static str, Vec<u8>),
    /// An empty tree under this name.
    EmptyTree(&'static str),
}

/// The board's head moved to a root of the README, `tasks` (holding task
/// `a`, unless `tasks` is false) and `rows`, made the same in the remote and
/// bob's clone, as git's transport refuses to send some such trees.
fn board_with(repos: &Repos, tasks: bool, rows: &[Root]) -> BoardHead {
    let mut head = None;
    for repo in [repos.remote(), repos.bob.clone()] {
        let readme = blob_in(&repo, README);
        let mut root = vec![("100644", "README.md", readme)];
        if tasks {
            let task = blob_in(&repo, &encode(&task("a")).unwrap());
            root.push((
                "40000",
                "tasks",
                raw_rows(&repo, &[("100644", "a.md", task)]),
            ));
        }
        for row in rows {
            root.push(match row {
                Root::Format(entries) => {
                    let entries: Vec<(&str, &str, String)> = entries
                        .iter()
                        .map(|(mode, name, object)| {
                            let oid = match object {
                                Object::Blob(bytes) => blob_in(&repo, bytes),
                                Object::EmptyTree | Object::Absent(_) => raw_rows(&repo, &[]),
                            };
                            (*mode, *name, oid)
                        })
                        .collect();
                    ("40000", "format", raw_rows(&repo, &entries))
                }
                Root::File(mode, name, bytes) | Root::ModeOverBlob(mode, name, bytes) => {
                    (*mode, *name, blob_in(&repo, bytes))
                }
                Root::EmptyTree(name) => ("40000", *name, raw_rows(&repo, &[])),
            });
        }
        root.sort_by_key(|(mode, name, _)| match *mode {
            "40000" => format!("{name}/"),
            _ => (*name).to_string(),
        });
        let commit = raw_commit(&repo, &raw_rows(&repo, &root));
        if let Some(head) = &head {
            assert_eq!(&commit, head, "the same commit in each repository");
        }
        head = Some(commit);
    }
    let head = head.unwrap();
    sh(&repos.remote(), &["update-ref", BOARD_REF, &head.0]);
    sh(
        &repos.bob,
        &["update-ref", "refs/remotes/origin/quecto/board", &head.0],
    );
    head
}

fn marker(entries: Vec<(&'static str, &'static str, Object)>) -> Root {
    Root::Format(entries)
}

fn board_file(bytes: &[u8]) -> Vec<(&'static str, &'static str, Object)> {
    vec![("100644", "board", Object::Blob(bytes.to_vec()))]
}

fn read(repos: &Repos, head: &BoardHead) -> Result<Arc<Board>, BoardStoreError> {
    store(&repos.bob).board_at(&head.0)
}

#[test]
fn this_quectos_marker_is_read() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let head = board_with(&repos, true, &[marker(board_file(FORMAT))]);
    assert_eq!(read(&repos, &head).unwrap().tasks.len(), 1);
}

/// A board of format 2 (tasks alone, before this quecto read the other
/// folders) is read as it is, and the next write leaves format 3.
#[test]
fn a_board_of_the_format_before_is_read_and_the_next_write_leaves_this_one() {
    assert_eq!(FORMAT, b"quecto-board 3\n");
    let repos = Repos::new();
    created(&store(&repos.alice));
    let head = board_with(&repos, true, &[marker(board_file(b"quecto-board 2\n"))]);
    let bob = store(&repos.bob);
    assert_eq!(bob.board_at(&head.0).unwrap().tasks.len(), 1);
    let moved = written(bob.write_tasks(&head, &[task("b")], "board: add b"));
    let marker = sh(
        &repos.remote(),
        &["cat-file", "blob", &format!("{}:format/board", moved.0)],
    );
    assert_eq!(marker + "\n", "quecto-board 3\n");
}

/// A board written before the marker (a README and `tasks`, no `format`)
/// is read as it is, and the next write adds the marker.
#[test]
fn a_board_without_a_format_marker_is_read_and_the_next_write_adds_it() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let head = board_with(&repos, true, &[]);
    let bob = store(&repos.bob);
    let moved = written(bob.write_tasks(&head, &[task("b")], "board: add b"));
    assert_eq!(
        sh(
            &repos.remote(),
            &["cat-file", "blob", &format!("{}:format/board", moved.0)]
        ) + "\n",
        String::from_utf8_lossy(FORMAT)
    );
}

/// A newer format asks for an upgrade whatever else its marker holds, and
/// before anything else this quecto would make of the board.
#[test]
fn a_newer_format_asks_for_an_upgrade_before_anything_else() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let mut padded = b"quecto-board 4\nwith more\n".to_vec();
    padded.resize(4 * 1024, b'x');
    for entries in [
        board_file(b"quecto-board 4\n"),
        vec![
            ("100755", "board", Object::Blob(padded.clone())),
            ("100644", "more", Object::Blob(b"x".to_vec())),
        ],
    ] {
        // A root this quecto takes for a clash, beside the newer marker.
        let head = board_with(
            &repos,
            true,
            &[
                marker(entries),
                Root::ModeOverBlob("40000", "src", b"code".to_vec()),
            ],
        );
        let read = read(&repos, &head);
        assert!(
            matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("upgrade quecto")),
            "{read:?}"
        );
    }
    // Past what a u64 holds, it is still newer.
    let head = board_with(
        &repos,
        true,
        &[marker(board_file(
            b"quecto-board 99999999999999999999999\n",
        ))],
    );
    let read = read(&repos, &head);
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("upgrade quecto")),
        "{read:?}"
    );
    // Before a task read refuses too: past the task-file bytes it takes.
    let head = board_with(&repos, true, &[marker(board_file(b"quecto-board 4\n"))]);
    let mut bob = store(&repos.bob);
    bob.limits.board_bytes = 1;
    let read = bob.board_at(&head.0);
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("upgrade quecto")),
        "{read:?}"
    );
}

/// A marker no quecto writes, beside a README and `tasks`, is damage to a
/// board: it asks for the board to be moved back, never for a rename.
#[test]
fn a_damaged_marker_asks_for_the_board_to_be_moved_back() {
    let mut long = FORMAT.to_vec();
    long.resize(100, b'x');
    let cases: Vec<(&str, Vec<Root>)> = vec![
        (
            "another name",
            vec![marker(vec![(
                "100644",
                "borad",
                Object::Blob(FORMAT.to_vec()),
            )])],
        ),
        (
            "an executable",
            vec![marker(vec![(
                "100755",
                "board",
                Object::Blob(FORMAT.to_vec()),
            )])],
        ),
        (
            "a link",
            vec![marker(vec![(
                "120000",
                "board",
                Object::Blob(FORMAT.to_vec()),
            )])],
        ),
        (
            "two entries",
            vec![marker(vec![
                ("100644", "board", Object::Blob(FORMAT.to_vec())),
                ("100644", "other", Object::Blob(b"x".to_vec())),
            ])],
        ),
        ("over 64 bytes", vec![marker(board_file(&long))]),
        (
            "a tree for its file",
            vec![marker(vec![("40000", "board", Object::EmptyTree)])],
        ),
        ("format 1", vec![marker(board_file(b"quecto-board 1\n"))]),
        (
            "format 2 and more",
            vec![marker(board_file(b"quecto-board 2\nmore\n"))],
        ),
        (
            "a leading zero",
            vec![marker(board_file(b"quecto-board 02\n"))],
        ),
        (
            "a leading zero before a newer number",
            vec![marker(board_file(b"quecto-board 03\n"))],
        ),
        ("a sign", vec![marker(board_file(b"quecto-board +3\n"))]),
        (
            "two format trees",
            vec![marker(board_file(FORMAT)), marker(board_file(FORMAT))],
        ),
        (
            "a format file beside the tree",
            vec![
                marker(board_file(FORMAT)),
                Root::File("100644", "format", FORMAT.to_vec()),
            ],
        ),
        (
            "tasks twice",
            vec![marker(board_file(FORMAT)), Root::EmptyTree("tasks")],
        ),
        ("no format", vec![marker(board_file(b"board"))]),
        (
            "a tree mode over a blob",
            vec![Root::ModeOverBlob("40000", "format", FORMAT.to_vec())],
        ),
    ];
    for (case, rows) in cases {
        let repos = Repos::new();
        created(&store(&repos.alice));
        let head = board_with(&repos, true, &rows);
        let read = read(&repos, &head);
        assert!(
            matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("hand edit") && why.contains(RECOVERY)),
            "{case}: {read:?}"
        );
    }
}

/// A marker alone, with neither a README nor `tasks`, is no board.
#[test]
fn a_marker_alone_is_no_board() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let remote = repos.remote();
    let marker = blob_in(&remote, FORMAT);
    let format = raw_rows(&remote, &[("100644", "board", marker)]);
    let root = raw_rows(&remote, &[("40000", "format", format)]);
    let head = raw_commit(&remote, &root);
    sh(&remote, &["update-ref", BOARD_REF, &head.0]);
    let read = store(&repos.bob).read();
    assert!(matches!(&read, Err(BoardStoreError::Clash(_))), "{read:?}");
}

/// A file named `format` at the root is a stray, as on a board before the
/// marker: the board is read, and the next write drops it for the marker.
#[test]
fn a_file_named_format_at_the_root_is_a_stray() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let head = board_with(
        &repos,
        true,
        &[Root::File("100644", "format", FORMAT.to_vec())],
    );
    let bob = store(&repos.bob);
    assert_eq!(bob.board_at(&head.0).unwrap().tasks.len(), 1);
    let moved = written(bob.write_tasks(&head, &[task("b")], "board: add b"));
    assert_eq!(
        sh(&repos.remote(), &["ls-tree", "-r", "--name-only", &moved.0]),
        "README.md\nformat/board\ntasks/a.md\ntasks/b.md"
    );
}

/// The board shape the check as merged before the marker took (master
/// `0723d9ee6`, `board_tree.rs` `board_shape`): a root entry is a file, or
/// the `tasks` tree; the root holds a README or `tasks`. Kept here so the
/// marker's promise stays checked: that quecto takes a marked root for a
/// clash, so it never reads, and never writes over, a board whose tasks it
/// would not see.
fn merged_board_shape(root: &[(&str, &str)]) -> Result<(), String> {
    let fits = |(name, kind): &(&str, &str)| match *name {
        "tasks" => *kind == "tree",
        _ => *kind == "blob",
    };
    let ours = |(name, _): &(&str, &str)| matches!(*name, "README.md" | "tasks");
    match (
        root.iter().any(ours),
        root.iter().find(|entry| !fits(entry)),
    ) {
        (true, None) => Ok(()),
        (false, None) => Err("neither a README.md nor a tasks tree".into()),
        (_, Some((name, kind))) => Err(format!("`{name}` is a {kind}: a clash")),
    }
}

#[test]
fn the_merged_board_shape_takes_a_marked_root_for_a_clash() {
    let legacy = [("README.md", "blob"), ("tasks", "tree")];
    assert_eq!(merged_board_shape(&legacy), Ok(()));
    let marked = [("README.md", "blob"), ("format", "tree"), ("tasks", "tree")];
    assert!(merged_board_shape(&marked).is_err());
    // And a write of this quecto leaves such a root.
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    let moved = written(alice.write_tasks(&head, &[task("a")], "board: add a"));
    let root = sh(&repos.remote(), &["ls-tree", &moved.0]);
    let root: Vec<(&str, &str)> = root
        .lines()
        .map(|row| {
            let (meta, name) = row.split_once('\t').unwrap();
            (name, meta.split(' ').nth(1).unwrap())
        })
        .collect();
    assert_eq!(root, marked);
}

/// A marker larger than this quecto reads to find its first line (64
/// KiB) may be a newer quecto's or a hand edit's: the refusal names both.
#[test]
fn a_marker_past_what_is_read_names_an_upgrade_and_a_mend() {
    let repos = Repos::new();
    created(&store(&repos.alice));
    let mut huge = b"quecto-board 4\n".to_vec();
    huge.resize(65 * 1024, b'x');
    let head = board_with(&repos, true, &[marker(board_file(&huge))]);
    let read = read(&repos, &head);
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("upgrade quecto") && why.contains(RECOVERY)),
        "{read:?}"
    );
}

/// A board this quecto does not read (a newer format, a damaged marker)
/// is never written, cleared or bootstrapped over.
#[test]
fn a_marker_this_quecto_does_not_read_refuses_every_change() {
    for bytes in [&b"quecto-board 4\n"[..], b"quecto-board 1\n"] {
        let repos = Repos::new();
        created(&store(&repos.alice));
        let head = board_with(&repos, true, &[marker(board_file(bytes))]);
        let bob = store(&repos.bob);
        let write = bob.write_tasks(&head, &[task("b")], "board: add b");
        assert!(
            matches!(write, Err(BoardStoreError::Unavailable(_))),
            "{write:?}"
        );
        let clear = bob.clear_set_aside(
            &head,
            &crate::application::project_board::ports::SetAsideClear {
                path: "tasks/a.md".into(),
                object: "0".repeat(40),
                repeated: Default::default(),
            },
            "board: clear",
        );
        assert!(
            matches!(clear, Err(BoardStoreError::Unavailable(_))),
            "{clear:?}"
        );
        let bootstrap = bob.bootstrap();
        assert!(
            matches!(bootstrap, Err(BoardStoreError::Unavailable(_))),
            "{bootstrap:?}"
        );
        assert_eq!(sh(&repos.remote(), &["rev-parse", BOARD_REF]), head.0);
    }
}
