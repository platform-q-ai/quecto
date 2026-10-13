//! Board trees as anyone with push access might write them: every part
//! that is not a task file the store wrote is set aside, never trusted, and
//! never stops a write.
use super::*;
use crate::application::project_board::ports::BoardListings;
use crate::domain::project_board::value_objects::shown_text::shown_char;
use crate::infrastructure::persistence::project_board::board_tree::{
    MAX_READ_ENTRIES, MAX_TASK_FILE_BYTES,
};
use crate::infrastructure::persistence::project_board::git::MAX_QUOTED_CHARS;
use crate::infrastructure::persistence::project_board::index::{BoardIndex, Refreshed};
use crate::infrastructure::persistence::project_board::indexed_listings::IndexedBoardListings;
use std::sync::Arc;

/// How the README and a refusal tell someone to undo a hand edit.
const RECOVERY: &str = "move `quecto/board` back to the commit before that edit";

#[test]
fn only_regular_task_files_are_read_and_the_rest_is_set_aside_by_id() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write_tasks(&head, &[task("good")], "board: add good"));
    let remote = repos.remote();
    let blob = |bytes: &[u8]| {
        format!(
            "100644 blob {}",
            sh_with(&remote, &["hash-object", "-w", "--stdin"], bytes)
        )
    };
    let mut oversize = encode(&task("big")).unwrap();
    oversize.resize(MAX_TASK_FILE_BYTES + 1, b' ');
    let target = sh_with(
        &remote,
        &["hash-object", "-w", "--stdin"],
        b"../../etc/passwd",
    );
    let commit = sh(&remote, &["rev-parse", BOARD_REF]);
    let empty = sh_with(&remote, &["mktree"], b"");
    hand_edit(
        &remote,
        &[
            ("broken.md", blob(b"{")),
            ("Upper.md", blob(&encode(&task("upper")).unwrap())),
            ("named.md", blob(&encode(&task("other")).unwrap())),
            ("big.md", blob(&oversize)),
            ("notes.txt", blob(b"notes")),
            (
                "exec.md",
                blob(&encode(&task("exec")).unwrap()).replace("100644", "100755"),
            ),
            ("link.md", format!("120000 blob {target}")),
            ("module.md", format!("160000 commit {commit}")),
            ("sub", format!("040000 tree {empty}")),
        ],
    );
    let bob = store(&repos.bob);
    let snapshot = bob.read().unwrap();
    assert_eq!(ids(&snapshot), vec!["good"]);
    let found: BTreeMap<&str, (Option<&str>, &str)> = snapshot
        .quarantined
        .iter()
        .map(|q| {
            (
                q.path.as_str(),
                (q.id().map(Slug::as_str), q.reason.as_str()),
            )
        })
        .collect();
    let by_id: Vec<(&str, Option<&str>)> =
        found.iter().map(|(path, (id, _))| (*path, *id)).collect();
    assert_eq!(
        by_id,
        [
            ("tasks/Upper.md", None),
            ("tasks/big.md", Some("big")),
            ("tasks/broken.md", Some("broken")),
            ("tasks/exec.md", Some("exec")),
            ("tasks/link.md", Some("link")),
            ("tasks/module.md", None),
            ("tasks/named.md", Some("named")),
            ("tasks/notes.txt", None),
            ("tasks/sub", None),
        ]
    );
    assert!(found["tasks/big.md"].1.contains("bytes"), "{found:?}");
    assert!(found["tasks/named.md"].1.contains("\"other\""), "{found:?}");
    assert!(found["tasks/broken.md"].1.contains("marker"), "{found:?}");
    assert!(found["tasks/link.md"].1.contains("regular"), "{found:?}");

    for id in ["broken", "link", "named", "exec"] {
        let refused = bob.write_tasks(&snapshot.head, &[task(id)], &format!("board: write {id}"));
        assert!(
            matches!(refused, Ok(BoardWrite::Refused(_))),
            "{id}: {refused:?}"
        );
    }
    assert_eq!(sh(&remote, &["rev-parse", BOARD_REF]), snapshot.head.0);
    // A write carries forward every task file it did not read, as it is
    // and with its own mode: links, executables and names a later quecto
    // may take as ids included. Strays (other names, subtrees, gitlinks)
    // are dropped.
    let moved = written(bob.write_tasks(&snapshot.head, &[task("fresh")], "board: add fresh"));
    let listing = |commit: &str| {
        sh(&remote, &["ls-tree", &format!("{commit}:tasks")])
            .lines()
            .map(|row| {
                let (meta, name) = row.split_once('\t').unwrap();
                (name.to_string(), meta.to_string())
            })
            .collect::<BTreeMap<String, String>>()
    };
    let (before, after) = (listing(&snapshot.head.0), listing(&moved.0));
    assert_eq!(
        after.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "Upper.md",
            "big.md",
            "broken.md",
            "exec.md",
            "fresh.md",
            "good.md",
            "link.md",
            "named.md"
        ]
    );
    for name in [
        "Upper.md",
        "big.md",
        "broken.md",
        "exec.md",
        "link.md",
        "named.md",
    ] {
        assert_eq!(after[name], before[name], "{name} carried as it was");
    }
    assert!(after["exec.md"].starts_with("100755 blob "));
    assert!(after["link.md"].starts_with("120000 blob "));
}

#[test]
fn a_board_tree_past_its_entry_bound_is_not_read() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    written(alice.write_tasks(&alice.read().unwrap().head, &[task("a")], "board: add a"));
    let blob = sh_with(&repos.remote(), &["hash-object", "-w", "--stdin"], b"x");
    let many: Vec<(String, String)> = (0..MAX_READ_ENTRIES)
        .map(|n| (format!("n{n}.md"), format!("100644 blob {blob}")))
        .collect();
    let many: Vec<(&str, String)> = many
        .iter()
        .map(|(name, meta)| (name.as_str(), meta.clone()))
        .collect();
    hand_edit(&repos.remote(), &many);
    let read = store(&repos.bob).read();
    assert!(
        matches!(&read, Err(BoardStoreError::TooLarge(why)) if why.contains("entries")),
        "{read:?}"
    );
}

/// What a raw tree entry names: a blob of these bytes, the empty tree, or
/// a blob of these bytes that no repository holds.
enum Object {
    Blob(Vec<u8>),
    EmptyTree,
    Absent(Vec<u8>),
}

/// A raw `tasks/` tree of exactly `entries` (mode, name, object), written
/// as git would never write one, and a board commit of it on the remote.
/// Each of `clones` gets the same objects, and its tracking ref the same
/// board, as if it had fetched it: git refuses to send some such trees over
/// the wire, so only a repository that already holds one (one sharing
/// objects, say) ever reads it.
fn raw_board(remote: &Path, entries: &[(&str, &str, Object)], clones: &[&Path]) -> BoardHead {
    let named: Vec<(&str, &[u8], &Object)> = entries
        .iter()
        .map(|(mode, name, object)| (*mode, name.as_bytes(), object))
        .collect();
    raw_board_named(remote, &named, clones)
}

/// [`raw_board`] with names of any bytes.
fn raw_board_named(
    remote: &Path,
    entries: &[(&str, &[u8], &Object)],
    clones: &[&Path],
) -> BoardHead {
    let head = raw_board_in(remote, entries);
    for clone in clones {
        assert_eq!(raw_board_in(clone, entries), head);
        // As if the clone had fetched it.
        sh(
            clone,
            &["update-ref", "refs/remotes/origin/quecto/board", &head.0],
        );
    }
    sh(remote, &["update-ref", BOARD_REF, &head.0]);
    head
}

/// A board commit of a legacy root, the README and `tasks` without the
/// format marker (`store_format_tests` makes marked roots).
fn raw_board_in(repo: &Path, entries: &[(&str, &[u8], &Object)]) -> BoardHead {
    let tasks = raw_tree_named(repo, entries);
    let readme = sh_with(repo, &["hash-object", "-w", "--stdin"], README);
    let root = format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n");
    let root = sh_with(repo, &["mktree"], root.as_bytes());
    raw_commit(repo, &root)
}

/// A tree of exactly `entries`, written raw, as git would never write one.
fn raw_tree(repo: &Path, entries: &[(&str, &str, Object)]) -> String {
    let named: Vec<(&str, &[u8], &Object)> = entries
        .iter()
        .map(|(mode, name, object)| (*mode, name.as_bytes(), object))
        .collect();
    raw_tree_named(repo, &named)
}

fn raw_tree_named(repo: &Path, entries: &[(&str, &[u8], &Object)]) -> String {
    let mut bytes = vec![];
    for (mode, name, object) in entries {
        let oid = match object {
            Object::Blob(content) => sh_with(repo, &["hash-object", "-w", "--stdin"], content),
            Object::EmptyTree => sh_with(repo, &["mktree"], b""),
            Object::Absent(content) => sh_with(repo, &["hash-object", "--stdin"], content),
        };
        bytes.extend(format!("{mode} ").as_bytes());
        bytes.extend(*name);
        bytes.push(0);
        let raw: Vec<u8> = (0..oid.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&oid[at..at + 2], 16).unwrap())
            .collect();
        bytes.extend(raw);
    }
    sh_with(
        repo,
        &["hash-object", "-t", "tree", "--literally", "-w", "--stdin"],
        &bytes,
    )
}

fn raw_commit(repo: &Path, root: &str) -> BoardHead {
    // A fixed date, so every repository makes the same commit.
    let at = "2026-10-10T08:00:00Z";
    let commit = Command::new("git")
        .current_dir(repo)
        .args(["commit-tree", root, "-m", "hand edit"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .envs([
            ("GIT_AUTHOR_NAME", "T"),
            ("GIT_AUTHOR_EMAIL", "t@x.io"),
            ("GIT_AUTHOR_DATE", at),
        ])
        .envs([
            ("GIT_COMMITTER_NAME", "T"),
            ("GIT_COMMITTER_EMAIL", "t@x.io"),
            ("GIT_COMMITTER_DATE", at),
        ])
        .output()
        .unwrap();
    assert!(
        commit.status.success(),
        "{}",
        String::from_utf8_lossy(&commit.stderr)
    );
    BoardHead(String::from_utf8_lossy(&commit.stdout).trim().to_string())
}

fn paths(snapshot: &BoardSnapshot) -> Vec<(&str, Option<&str>)> {
    snapshot
        .quarantined
        .iter()
        .map(|q| (q.path.as_str(), q.id().map(Slug::as_str)))
        .collect()
}

/// git's own transport refuses this shape (upload-pack: "entry has blob
/// mode, but is not a blob"), so it reaches a store only through objects
/// it already holds; the store still never trusts a mode over a type. A
/// task file of that shape is set aside and dropped by the next write
/// (carrying it would make a board no one can fetch).
#[test]
fn a_file_mode_over_a_tree_object_is_set_aside_and_never_stops_a_write() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let good = encode(&task("good")).unwrap();
    let head = raw_board(
        &repos.remote(),
        &[
            ("100644", "good.md", Object::Blob(good.clone())),
            ("100644", "tree.md", Object::EmptyTree),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let board = bob.board_at(&head.0).unwrap();
    let paths: Vec<(&str, Option<&str>)> = board
        .quarantined
        .iter()
        .map(|q| (q.path.as_str(), q.id().map(Slug::as_str)))
        .collect();
    assert_eq!(paths, [("tasks/tree.md", Some("tree"))]);
    assert_eq!(
        board.tasks.keys().map(Slug::as_str).collect::<Vec<_>>(),
        ["good"]
    );
    let refused = bob.write_tasks(&head, &[task("tree")], "board: write tree");
    assert!(matches!(refused, Ok(BoardWrite::Refused(_))), "{refused:?}");
    let moved = written(bob.write_tasks(&head, &[task("fresh")], "board: add fresh"));
    let kept = sh(&repos.remote(), &["ls-tree", "-r", &moved.0]);
    let kept: Vec<(&str, &str)> = kept
        .lines()
        .map(|row| {
            let (meta, name) = row.split_once('\t').unwrap();
            (name, &meta[..12])
        })
        .collect();
    assert_eq!(
        kept,
        [
            ("README.md", "100644 blob "),
            ("format/board", "100644 blob "),
            ("tasks/fresh.md", "100644 blob "),
            ("tasks/good.md", "100644 blob "),
        ]
    );
    assert_eq!(
        sh(
            &repos.remote(),
            &["cat-file", "blob", &format!("{}:tasks/good.md", moved.0)]
        ) + "\n",
        String::from_utf8(good).unwrap()
    );
}

/// A task file named twice in one tree: git itself rejects such a tree
/// (`fsck`: duplicateEntries, which GitHub and any remote with
/// `receive.fsckObjects` run on every push), so carrying both forward
/// would make every later push fail, and keeping one would silently drop
/// the other. Every write is refused, naming the file, and the board stays
/// as it is until someone mends it.
#[test]
fn a_task_file_named_twice_refuses_every_write_and_drops_neither() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let mut other = task("dup").into_fields();
    other.title = "Another".into();
    let other = Task::new(other).unwrap();
    raw_board(
        &repos.remote(),
        &[
            (
                "100644",
                "dup.md",
                Object::Blob(encode(&task("dup")).unwrap()),
            ),
            ("100644", "dup.md", Object::Blob(encode(&other).unwrap())),
            (
                "100644",
                "ok.md",
                Object::Blob(encode(&task("ok")).unwrap()),
            ),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let snapshot = bob.read().unwrap();
    assert_eq!(ids(&snapshot), ["ok"]);
    assert_eq!(
        paths(&snapshot),
        [("tasks/dup.md", Some("dup")), ("tasks/dup.md", Some("dup"))]
    );
    let blobs: Vec<String> = [encode(&task("dup")).unwrap(), encode(&other).unwrap()]
        .iter()
        .map(|bytes| sh_with(&repos.bob, &["hash-object", "--stdin"], bytes))
        .collect();
    for id in ["dup", "ok", "fresh"] {
        let refused = bob.write_tasks(&snapshot.head, &[task(id)], &format!("board: write {id}"));
        let Ok(BoardWrite::Refused(why)) = &refused else {
            panic!("{id}: {refused:?}");
        };
        assert!(
            why.contains("tasks/dup.md") && why.contains("more than once"),
            "{why}"
        );
        // Both copies, so whoever mends the board can choose, and how to.
        assert!(
            blobs.iter().all(|blob| why.contains(blob.as_str())),
            "{why}"
        );
        assert!(why.contains(RECOVERY), "{why}");
    }
    assert!(
        String::from_utf8_lossy(README).contains(RECOVERY),
        "the README gives the same way back"
    );
    assert_eq!(
        sh(&repos.remote(), &["rev-parse", BOARD_REF]),
        snapshot.head.0
    );
}

/// The root's `tasks` entry is trusted only for what it really is: a
/// directory mode over a blob is not a board.
#[test]
fn a_tasks_entry_that_is_not_a_tree_is_not_a_board() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let mut heads = vec![];
    for repo in [&repos.remote(), &repos.bob] {
        let root = raw_tree(
            repo,
            &[
                ("100644", "README.md", Object::Blob(README.to_vec())),
                ("40000", "tasks", Object::Blob(b"not a tree".to_vec())),
            ],
        );
        heads.push(raw_commit(repo, &root));
    }
    assert_eq!(heads[0], heads[1]);
    let read = store(&repos.bob).board_at(&heads[1].0);
    assert!(
        matches!(&read, Err(BoardStoreError::Clash(why)) if why.contains("tasks")),
        "{read:?}"
    );
}

/// An object the tree names but no repository holds is an error, never a
/// file set aside as "not regular".
#[test]
fn an_object_missing_everywhere_is_an_error_not_a_file_set_aside() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let absent = b"held by no repository".to_vec();
    let oid = sh_with(&repos.bob, &["hash-object", "--stdin"], &absent);
    let head = raw_board(
        &repos.remote(),
        &[
            (
                "100644",
                "ok.md",
                Object::Blob(encode(&task("ok")).unwrap()),
            ),
            ("100644", "gone.md", Object::Absent(absent)),
        ],
        &[&repos.bob],
    );
    let read = store(&repos.bob).board_at(&head.0);
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains(&oid) && why.contains("missing")),
        "{read:?}"
    );
}

#[test]
fn a_write_past_the_task_bound_is_refused_before_anything_is_built() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let remote = repos.remote();
    let files = repos.dir.path().join("files");
    std::fs::create_dir(&files).unwrap();
    let mut paths = vec![];
    for n in 0..MAX_TASK_ENTRIES {
        let name = format!("t{n}.md");
        std::fs::write(files.join(&name), encode(&task(&format!("t{n}"))).unwrap()).unwrap();
        paths.push(name);
    }
    let oids = sh_with(
        &remote,
        &["hash-object", "-w", "--stdin-paths"],
        paths
            .iter()
            .map(|name| format!("{}\n", files.join(name).display()))
            .collect::<String>()
            .as_bytes(),
    );
    let entries: String = paths
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
    let commit = sh(
        &remote,
        &["commit-tree", &root, "-p", &head, "-m", "full board"],
    );
    sh(&remote, &["update-ref", BOARD_REF, &commit]);
    let bob = store(&repos.bob);
    let full = bob.read().unwrap();
    assert_eq!(full.tasks.len(), MAX_TASK_ENTRIES);
    let refused = bob.write_tasks(&full.head, &[task("one-more")], "board: add one more");
    assert!(
        matches!(&refused, Err(BoardStoreError::InvalidWrite(why)) if why.contains("at most")),
        "{refused:?}"
    );
    let renamed = {
        let mut fields = task("t0").into_fields();
        fields.title = "Renamed".into();
        Task::new(fields).unwrap()
    };
    let moved = written(bob.write_tasks(&full.head, &[renamed], "board: rename t0"));
    assert_eq!(
        store(&repos.alice).read().unwrap().head,
        moved,
        "a full board still takes changes"
    );
}

/// A task file whose cell holds an escape sequence, a right-to-left
/// override, an ideographic space or a long run of text is set aside with
/// a reason that never quotes the file: only what shows as itself, short,
/// so the index takes the reason and every listing answers, with no
/// rebuild after the first.
#[test]
fn a_hostile_cell_never_reaches_a_reason_or_stops_a_listing() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write_tasks(&head, &[task("good")], "board: add good"));
    let long = "k".repeat(3 * 1024);
    let hostile = |id: &str, word: &str| {
        let text = String::from_utf8(encode(&task(id)).unwrap()).unwrap();
        let planted = text.replacen("| Status | ready |", &format!("| Status | {word} |"), 1);
        assert_ne!(planted, text, "the status cell is planted");
        Object::Blob(planted.into_bytes())
    };
    let good = Object::Blob(encode(&task("good")).unwrap());
    raw_board(
        &repos.remote(),
        &[
            ("100644", "good.md", good),
            ("100644", "esc.md", hostile("esc", "x\u{1b}[31m")),
            ("100644", "long.md", hostile("long", &long)),
            ("100644", "rtl.md", hostile("rtl", "x\u{202e}y")),
            ("100644", "wide.md", hostile("wide", "x\u{3000}y")),
        ],
        &[],
    );
    let bob = Arc::new(store(&repos.bob));
    let snapshot = bob.read().unwrap();
    assert_eq!(snapshot.quarantined.len(), 4, "{snapshot:?}");
    for file in &snapshot.quarantined {
        assert!(
            file.reason.chars().all(|c| shown_char(c) || c == ' '),
            "{:?}",
            file.reason
        );
        assert!(!file.reason.contains("kkkk"), "{:?}", file.reason);
        assert!(file.reason.chars().count() <= MAX_QUOTED_CHARS + 2);
    }
    let index = BoardIndex::of(&bob).unwrap();
    let listings = IndexedBoardListings::new(index, bob.clone(), bob.clone());
    for _ in 0..2 {
        let ready = listings.ready().unwrap();
        assert_eq!(ready.quarantined.len(), 4);
    }
    let index = BoardIndex::of(&bob).unwrap();
    assert_eq!(
        index.refresh(&*bob, &*bob),
        Ok(Refreshed::Unchanged {
            head: snapshot.head.0.clone()
        }),
        "the index holds: no rebuild loop"
    );
    let listed = index.ready();
    assert!(
        listed.is_ok(),
        "the index answers its own query: {listed:?}"
    );
}

#[path = "store_clear_tests.rs"]
mod clear;
#[path = "store_folders_tests.rs"]
mod folders;
#[path = "store_format_tests.rs"]
mod format;
#[path = "store_legacy_tests.rs"]
mod legacy;
#[path = "store_names_tests.rs"]
mod names;
