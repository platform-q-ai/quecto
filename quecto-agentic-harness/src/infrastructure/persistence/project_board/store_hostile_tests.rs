//! Board trees as anyone with push access might write them: every part
//! that is not a task file the store wrote is set aside, never trusted, and
//! never stops a write.
use super::*;
use crate::infrastructure::persistence::project_board::task_record::MAX_TASK_FILE_BYTES;

#[test]
fn only_regular_task_files_are_read_and_the_rest_is_set_aside_by_id() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    let head = created(&alice);
    written(alice.write(&head, &[task("good")], "board: add good"));
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
            ("broken.json", blob(b"{")),
            ("Upper.json", blob(&encode(&task("upper")).unwrap())),
            ("named.json", blob(&encode(&task("other")).unwrap())),
            ("big.json", blob(&oversize)),
            ("notes.txt", blob(b"notes")),
            (
                "exec.json",
                blob(&encode(&task("exec")).unwrap()).replace("100644", "100755"),
            ),
            ("link.json", format!("120000 blob {target}")),
            ("module.json", format!("160000 commit {commit}")),
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
                (q.id.as_ref().map(Slug::as_str), q.reason.as_str()),
            )
        })
        .collect();
    let by_id: Vec<(&str, Option<&str>)> =
        found.iter().map(|(path, (id, _))| (*path, *id)).collect();
    assert_eq!(
        by_id,
        [
            ("tasks/Upper.json", None),
            ("tasks/big.json", Some("big")),
            ("tasks/broken.json", Some("broken")),
            ("tasks/exec.json", Some("exec")),
            ("tasks/link.json", Some("link")),
            ("tasks/module.json", Some("module")),
            ("tasks/named.json", Some("named")),
            ("tasks/notes.txt", None),
            ("tasks/sub", None),
        ]
    );
    assert!(found["tasks/big.json"].1.contains("bytes"), "{found:?}");
    assert!(
        found["tasks/named.json"].1.contains("\"other\""),
        "{found:?}"
    );
    assert!(
        found["tasks/broken.json"].1.contains("malformed"),
        "{found:?}"
    );
    assert!(found["tasks/link.json"].1.contains("regular"), "{found:?}");

    for id in ["broken", "link", "named"] {
        let refused = bob.write(&snapshot.head, &[task(id)], &format!("board: write {id}"));
        assert!(
            matches!(refused, Ok(BoardWrite::Refused(_))),
            "{id}: {refused:?}"
        );
    }
    assert_eq!(sh(&remote, &["rev-parse", BOARD_REF]), snapshot.head.0);
    // A write carries forward only regular task files, never links,
    // modules, subtrees or strays.
    let moved = written(bob.write(&snapshot.head, &[task("fresh")], "board: add fresh"));
    let kept = sh(
        &remote,
        &["ls-tree", "--name-only", &format!("{}:tasks", moved.0)],
    );
    assert_eq!(
        kept.lines().collect::<Vec<_>>(),
        [
            "big.json",
            "broken.json",
            "fresh.json",
            "fresh.md",
            "good.json",
            "good.md",
            "named.json"
        ]
    );
}

#[test]
fn a_board_tree_past_its_entry_bound_is_not_read() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    written(alice.write(&alice.read().unwrap().head, &[task("a")], "board: add a"));
    let blob = sh_with(&repos.remote(), &["hash-object", "-w", "--stdin"], b"x");
    let many: Vec<(String, String)> = (0..MAX_TASK_ENTRIES)
        .map(|n| (format!("n{n}.md"), format!("100644 blob {blob}")))
        .collect();
    let many: Vec<(&str, String)> = many
        .iter()
        .map(|(name, meta)| (name.as_str(), meta.clone()))
        .collect();
    hand_edit(&repos.remote(), &many);
    let read = store(&repos.bob).read();
    assert!(
        matches!(&read, Err(BoardStoreError::Unavailable(why)) if why.contains("entries")),
        "{read:?}"
    );
}

/// What a raw tree entry names: a blob of these bytes, or the empty tree.
enum Object {
    Blob(Vec<u8>),
    EmptyTree,
}

/// A raw `tasks/` tree of exactly `entries` (mode, name, object), written
/// as git would never write one, and a board commit of it on the remote.
/// Each of `clones` gets the same objects too: git refuses to send such a
/// tree over the wire, so only a repository that already holds it (one
/// sharing objects, say) ever reads it.
fn raw_board(remote: &Path, entries: &[(&str, &str, Object)], clones: &[&Path]) -> BoardHead {
    let head = raw_board_in(remote, entries);
    for clone in clones {
        assert_eq!(raw_board_in(clone, entries), head);
    }
    sh(remote, &["update-ref", BOARD_REF, &head.0]);
    head
}

fn raw_board_in(repo: &Path, entries: &[(&str, &str, Object)]) -> BoardHead {
    let mut bytes = vec![];
    for (mode, name, object) in entries {
        let oid = match object {
            Object::Blob(content) => sh_with(repo, &["hash-object", "-w", "--stdin"], content),
            Object::EmptyTree => sh_with(repo, &["mktree"], b""),
        };
        bytes.extend(format!("{mode} {name}\0").as_bytes());
        let raw: Vec<u8> = (0..oid.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&oid[at..at + 2], 16).unwrap())
            .collect();
        bytes.extend(raw);
    }
    let tasks = sh_with(
        repo,
        &["hash-object", "-t", "tree", "--literally", "-w", "--stdin"],
        &bytes,
    );
    let readme = sh_with(repo, &["hash-object", "-w", "--stdin"], README);
    let root = format!("100644 blob {readme}\tREADME.md\n040000 tree {tasks}\ttasks\n");
    let root = sh_with(repo, &["mktree"], root.as_bytes());
    // A fixed date, so every repository makes the same commit.
    let at = "2026-10-10T08:00:00Z";
    let commit = Command::new("git")
        .current_dir(repo)
        .args(["commit-tree", &root, "-m", "hand edit"])
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
        .map(|q| (q.path.as_str(), q.id.as_ref().map(Slug::as_str)))
        .collect()
}

/// git's own transport refuses this shape (upload-pack: "entry has blob
/// mode, but is not a blob"), so it reaches a store only through objects
/// it already holds; the store still never trusts a mode over a type.
#[test]
fn a_file_mode_over_a_tree_object_is_set_aside_and_never_stops_a_write() {
    let repos = Repos::new();
    let alice = store(&repos.alice);
    created(&alice);
    let good = encode(&task("good")).unwrap();
    let head = raw_board(
        &repos.remote(),
        &[
            ("100644", "good.json", Object::Blob(good)),
            ("100644", "good.md", Object::EmptyTree),
            ("100644", "tree.json", Object::EmptyTree),
        ],
        &[&repos.bob],
    );
    let bob = store(&repos.bob);
    let board = bob.board_at(&head.0).unwrap();
    let paths: Vec<(&str, Option<&str>)> = board
        .quarantined
        .iter()
        .map(|q| (q.path.as_str(), q.id.as_ref().map(Slug::as_str)))
        .collect();
    assert_eq!(
        paths,
        [
            ("tasks/good.md", Some("good")),
            ("tasks/tree.json", Some("tree"))
        ]
    );
    assert!(
        board.tasks.is_empty(),
        "a task with a part set aside is not read"
    );
    let refused = bob.write(&head, &[task("good")], "board: write good");
    assert!(matches!(refused, Ok(BoardWrite::Refused(_))), "{refused:?}");
    let moved = written(bob.write(&head, &[task("fresh")], "board: add fresh"));
    let kept = sh(&repos.remote(), &["ls-tree", "-r", "--name-only", &moved.0]);
    assert_eq!(
        kept.lines().collect::<Vec<_>>(),
        ["README.md", "tasks/fresh.json", "tasks/fresh.md"]
    );
}

#[test]
fn a_name_listed_twice_sets_its_task_aside() {
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
                "dup.json",
                Object::Blob(encode(&task("dup")).unwrap()),
            ),
            ("100644", "dup.json", Object::Blob(encode(&other).unwrap())),
            (
                "100644",
                "ok.json",
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
        [
            ("tasks/dup.json", Some("dup")),
            ("tasks/dup.json", Some("dup"))
        ]
    );
    let refused = bob.write(&snapshot.head, &[task("dup")], "board: write dup");
    assert!(matches!(refused, Ok(BoardWrite::Refused(_))), "{refused:?}");
    let moved = written(bob.write(&snapshot.head, &[task("ok")], "board: write ok"));
    let kept = sh(
        &repos.remote(),
        &["ls-tree", "--name-only", &format!("{}:tasks", moved.0)],
    );
    assert_eq!(kept.lines().collect::<Vec<_>>(), ["ok.json", "ok.md"]);
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
    for n in 0..MAX_TASKS {
        let task = task(&format!("t{n}"));
        for (name, bytes) in [
            (format!("t{n}.json"), encode(&task).unwrap()),
            (format!("t{n}.md"), summary(&task).into_bytes()),
        ] {
            std::fs::write(files.join(&name), bytes).unwrap();
            paths.push(name);
        }
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
    assert_eq!(full.tasks.len(), MAX_TASKS);
    let refused = bob.write(&full.head, &[task("one-more")], "board: add one more");
    assert!(
        matches!(&refused, Err(BoardStoreError::InvalidWrite(why)) if why.contains("at most")),
        "{refused:?}"
    );
    let renamed = {
        let mut fields = task("t0").into_fields();
        fields.title = "Renamed".into();
        Task::new(fields).unwrap()
    };
    let moved = written(bob.write(&full.head, &[renamed], "board: rename t0"));
    assert_eq!(
        store(&repos.alice).read().unwrap().head,
        moved,
        "a full board still takes changes"
    );
}
