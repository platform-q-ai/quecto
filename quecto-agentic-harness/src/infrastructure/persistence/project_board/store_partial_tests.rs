//! A partial clone fetches only what a read reads (#2481 S10b): the
//! board's commits, its root, the archive's tree alone and never what lies
//! under it, and the live folders with every file but one past the largest
//! file cap, which is set aside and carried by its id, never fetched. A
//! full clone never becomes a partial one.
use super::*;

/// Whether `repo` holds `oid` itself, fetching nothing.
fn held(repo: &Path, oid: &str) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["cat-file", "-e", oid])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", repo.as_os_str()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".as_ref()),
            ("GIT_CONFIG_NOSYSTEM", "1".as_ref()),
            ("GIT_NO_LAZY_FETCH", "1".as_ref()),
        ])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// `remote` serves filtered fetches and objects by id.
fn serve_filters(remote: &Path) {
    for (key, value) in [
        ("uploadpack.allowFilter", "true"),
        ("uploadpack.allowAnySHA1InWant", "true"),
    ] {
        sh(remote, &["config", key, value]);
    }
}

/// `name`, a clone of the remote made with `filter`, whose remote serves
/// filtered fetches and objects by id.
fn partial(repos: &Repos, filter: &str, name: &str) -> PathBuf {
    let remote = repos.remote();
    serve_filters(&remote);
    let url = format!("file://{}", remote.display());
    let filter = format!("--filter={filter}");
    sh(repos.dir.path(), &["clone", "-q", &filter, &url, name]);
    repos.dir.path().join(name)
}

/// #2705 B9 review round 2 (F2): a clone made with no trees names a
/// file at an older revision of the board from that revision's root and
/// folder trees alone, fetched for it: never the file itself, nor any
/// other file of that revision.
#[test]
fn a_partial_clone_names_a_file_at_an_older_rev_from_its_trees_alone() {
    use crate::application::project_board::ports::ArtifactKey;
    let repos = Repos::new();
    let carol = partial(&repos, "tree:0", "carol");
    let alice = store(&repos.alice);
    let head = created(&alice);
    let first = written(alice.write_tasks(&head, &[task("a"), task("b")], "board: add a, b"));
    let mut renamed = task("a").into_fields();
    renamed.title = "Renamed".into();
    let renamed = Task::new(renamed).unwrap();
    let second = written(alice.write_tasks(&first, &[renamed], "board: rename a"));
    let partial = store(&carol);
    assert_eq!(partial.read().unwrap().head, second);
    let remote = repos.remote();
    let at =
        |rev: &BoardHead, path: &str| sh(&remote, &["rev-parse", &format!("{}:{path}", rev.0)]);
    let old = at(&first, "tasks/a.md");
    assert!(
        !held(&carol, &old),
        "the clone holds the head's files alone"
    );
    let key = ArtifactKey::Task(Slug::parse("a").unwrap());
    assert_eq!(partial.object_at(&first, &key).unwrap(), Some(old.clone()));
    assert!(held(&carol, &at(&first, "tasks")), "its folder's tree came");
    assert!(!held(&carol, &old), "the file itself never came");
    assert_eq!(
        partial.object_at(&second, &key).unwrap(),
        Some(at(&second, "tasks/a.md"))
    );
}

/// A clone made with no trees, before the board existed, fetches the
/// board's commits, its root, its folders and the archive's tree alone:
/// it reads every kind and a write carries the archive, and nothing under
/// the archive is ever fetched. A full clone's reads and writes leave it
/// a full clone.
#[test]
fn a_partial_clone_fetches_the_archive_tree_alone_and_carries_it() {
    let repos = Repos::new();
    let carol = partial(&repos, "tree:0", "carol");
    created(&store(&repos.alice));
    let head = board_of(&repos, |repo| {
        let deep = raw_rows(repo, &[("100644", "d.txt", blob_in(repo, b"deep"))]);
        let old = vec![
            ("100644", "old.txt".into(), blob_in(repo, b"anything")),
            ("40000", "deep".into(), deep),
        ];
        let mut rows = every_kind();
        rows.push(("40000", ARCHIVE, Node::Made(old)));
        rows
    });
    let remote = repos.remote();
    let under = [
        at(&remote, &head, "archive/old.txt"),
        at(&remote, &head, "archive/deep"),
    ];
    let partial = store(&carol);
    let board = partial.read().unwrap();
    assert_eq!(board.reviews, [review("a", 1)]);
    assert_eq!(board.discussions, [discussion("d")]);
    let moved = written(partial.write_tasks(&head, &[task("b")], "board: add b"));
    assert_eq!(at(&remote, &moved, ARCHIVE), at(&remote, &head, ARCHIVE));
    assert!(held(&carol, &at(&remote, &head, ARCHIVE)));
    for oid in &under {
        assert!(!held(&carol, oid), "{oid} under the archive was fetched");
    }
    let bob = store(&repos.bob);
    written(bob.write_tasks(&bob.read().unwrap().head, &[task("c")], "board: add c"));
    let promisor = Command::new("git")
        .current_dir(&repos.bob)
        .args(["config", "--get", "remote.origin.promisor"])
        .env_clear()
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("PATH", std::env::var_os("PATH").unwrap())
        .output()
        .unwrap();
    assert!(
        promisor.stdout.is_empty(),
        "a full clone stays one: {promisor:?}"
    );
}

/// A file past every cap is never fetched by a clone made without blobs:
/// it is set aside as too large and carried by its id. A write into its
/// folder rebuilds the folder around it, and a full clone reads the very
/// file there.
#[test]
fn a_blob_past_every_cap_is_set_aside_unfetched_and_carried_by_its_id() {
    let repos = fresh();
    let big = vec![b'x'; 2 * 1024 * 1024];
    let head = board_of(&repos, |_| {
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            (
                "40000",
                "plans",
                Node::Tree(vec![
                    regular("a.1.md", bytes::<PlanCodec>(&plan("a", 1))),
                    regular("a.2.md", big.clone()),
                ]),
            ),
        ]
    });
    let carol = partial(&repos, "blob:none", "carol");
    let remote = repos.remote();
    let oid = at(&remote, &head, "plans/a.2.md");
    let partial = store(&carol);
    let board = partial.read().unwrap();
    assert_eq!(board.plans, [plan("a", 1)]);
    let key = ArtifactKey::Plan(key_slug("a"), 2);
    assert_eq!(aside(&board), [("plans/a.2.md", Some(&key), true)]);
    assert_eq!(
        reason(&board, "plans/a.2.md"),
        "plan file is at least 1572865 bytes, more than a partial clone fetches; the most is \
         1572864"
    );
    assert!(!held(&carol, &oid), "the file past every cap was fetched");
    let three = [BoardArtifact::Plan(plan("a", 3))];
    let moved = written(partial.write(&head, &three, "board: plan a 3"));
    assert_eq!(
        names(&remote, &moved, "plans"),
        ["a.1.md", "a.2.md", "a.3.md"]
    );
    assert_eq!(at(&remote, &moved, "plans/a.2.md"), oid);
    assert!(
        !held(&carol, &oid),
        "a write fetched the file past every cap"
    );
    let board = store(&repos.alice).read().unwrap();
    assert_eq!(board.plans, [plan("a", 1), plan("a", 3)]);
    assert_eq!(
        reason(&board, "plans/a.2.md"),
        "plan file is 2097152 bytes; the most is 1572864"
    );
}

/// `repo`'s value of `key`, if it is set.
fn config(repo: &Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["config", "--get", key])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", repo.as_os_str()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".as_ref()),
            ("GIT_CONFIG_NOSYSTEM", "1".as_ref()),
        ])
        .output()
        .unwrap();
    let value = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string();
    output.status.success().then_some(value)
}

/// The board's remote runs `git-upload-pack` through a wrapper that counts
/// its runs, which this reads.
fn counted(repos: &Repos, clone: &Path) -> impl Fn() -> usize {
    let count = repos.dir.path().join("upload-packs");
    let wrapper = repos.dir.path().join("upload-pack");
    executable(
        &wrapper,
        &format!(
            "#!/bin/sh\necho x >> '{}'\nexec git-upload-pack \"$@\"\n",
            count.display()
        ),
    );
    sh(
        clone,
        &[
            "config",
            "remote.origin.uploadpack",
            wrapper.to_str().unwrap(),
        ],
    );
    move || {
        std::fs::read_to_string(&count)
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }
}

/// `plans/` holding plan 1 of task `a`, and a file `a.2.md` past every cap.
fn plans_past_a_cap(repos: &Repos) -> BoardHead {
    board_of(repos, |_| {
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            (
                "40000",
                "plans",
                Node::Tree(vec![
                    regular("a.1.md", bytes::<PlanCodec>(&plan("a", 1))),
                    regular("a.2.md", vec![b'x'; 2 * 1024 * 1024]),
                ]),
            ),
        ]
    })
}

/// A partial clone that has read the board reads a new head as one fetch
/// of what changed, however it next starts: a file the filter left on the
/// remote is known for one, never fetched again, and the folder holding it
/// never fetched whole again.
#[test]
fn a_new_head_is_one_fetch_of_what_changed() {
    let repos = fresh();
    let head = plans_past_a_cap(&repos);
    let carol = partial(&repos, "tree:0", "carol");
    let oid = at(&repos.remote(), &head, "plans/a.2.md");
    assert_eq!(store(&carol).read().unwrap().plans, [plan("a", 1)]);
    let fetches = counted(&repos, &carol);
    let alice = store(&repos.alice);
    let three = [BoardArtifact::Plan(plan("a", 3))];
    written(alice.write(&alice.read().unwrap().head, &three, "board: plan a 3"));
    let before = fetches();
    let board = store(&carol).read().unwrap();
    assert_eq!(board.plans, [plan("a", 1), plan("a", 3)]);
    assert!(reason(&board, "plans/a.2.md").contains("at least"));
    assert_eq!(
        fetches() - before,
        2,
        "a new head is the remote's head and one fetch"
    );
    assert!(!held(&carol, &oid), "the file past every cap was fetched");
}

/// A clone made partial by hand, either way git marks one, keeps its own
/// fetch filter (none) after the store's filtered fetches.
#[test]
fn a_filtered_fetch_leaves_the_clones_own_filter_as_it_was() {
    let repos = fresh();
    plans_past_a_cap(&repos);
    serve_filters(&repos.remote());
    let url = format!("file://{}", repos.remote().display());
    for (name, marks) in [
        ("flagged", vec![("remote.origin.promisor", "true")]),
        (
            "extended",
            vec![
                ("core.repositoryformatversion", "1"),
                ("extensions.partialClone", "origin"),
            ],
        ),
    ] {
        sh(repos.dir.path(), &["init", "-q", name]);
        let clone = repos.dir.path().join(name);
        sh(&clone, &["remote", "add", "origin", &url]);
        for (key, value) in marks {
            sh(&clone, &["config", key, value]);
        }
        let board = store(&clone).read().unwrap();
        assert_eq!(board.plans, [plan("a", 1)], "{name}");
        assert!(
            reason(&board, "plans/a.2.md").contains("at least"),
            "{name}"
        );
        assert_eq!(
            config(&clone, "remote.origin.partialclonefilter"),
            None,
            "{name}"
        );
    }
}

/// A server that refuses a filter, or knows none, is fetched from whole:
/// the read is the full clone's, a file past every cap told at its size.
#[test]
fn a_server_without_filters_is_fetched_from_whole() {
    for (key, value) in [
        ("uploadpackfilter.allow", "false"),
        ("uploadpack.allowFilter", "false"),
    ] {
        let repos = fresh();
        plans_past_a_cap(&repos);
        let carol = partial(&repos, "tree:0", "carol");
        sh(&repos.remote(), &["config", key, value]);
        let board = store(&carol).read().unwrap();
        assert_eq!(board.plans, [plan("a", 1)], "{key}");
        assert_eq!(
            reason(&board, "plans/a.2.md"),
            "plan file is 2097152 bytes; the most is 1572864",
            "{key}"
        );
    }
}

/// A full clone that lost a file is never told it is too large, nor made a
/// partial clone: it fetches the file whole, as before partial clones.
#[test]
fn a_full_clone_missing_a_file_fetches_it_whole() {
    let repos = fresh();
    let head = plans_past_a_cap(&repos);
    serve_filters(&repos.remote());
    let oid = at(&repos.remote(), &head, "plans/a.2.md");
    let loose = repos
        .bob
        .join(".git/objects")
        .join(&oid[..2])
        .join(&oid[2..]);
    std::fs::remove_file(loose).unwrap();
    let board = store(&repos.bob).read().unwrap();
    assert_eq!(
        reason(&board, "plans/a.2.md"),
        "plan file is 2097152 bytes; the most is 1572864"
    );
    assert!(held(&repos.bob, &oid));
    assert_eq!(config(&repos.bob, "remote.origin.promisor"), None);
    assert_eq!(config(&repos.bob, "remote.origin.partialclonefilter"), None);
}

/// A root entry a partial clone lacks is judged by its real object, as a
/// full clone judges it: a file's mode over a tree is no board in both.
#[test]
fn a_damaged_root_is_judged_alike_in_partial_and_full_clones() {
    let repos = fresh();
    board_of(&repos, |repo| {
        let inner = raw_rows(repo, &[("100644", "f", blob_in(repo, b"f"))]);
        vec![
            ("40000", "tasks", Node::Tree(vec![])),
            (
                "100644",
                "notes",
                Node::Made(vec![("100644", "f".into(), inner)]),
            ),
        ]
    });
    let carol = partial(&repos, "tree:0", "carol");
    let full = store(&repos.bob).read();
    assert!(matches!(&full, Err(BoardStoreError::Clash(_))), "{full:?}");
    assert_eq!(store(&carol).read(), full);
}

/// A format marker past what a read takes, which a partial clone's filter
/// left on the remote, is told as at least the filter's size there; a full
/// clone tells its size.
#[test]
fn a_marker_left_on_the_remote_is_told_at_least_its_size() {
    let repos = fresh();
    board_marked(&repos, &vec![b'x'; 2 * 1024 * 1024], |_| {
        vec![("40000", "tasks", Node::Tree(vec![]))]
    });
    let carol = partial(&repos, "blob:none", "carol");
    let told = |read: Result<BoardSnapshot, BoardStoreError>| match read {
        Err(BoardStoreError::Unavailable(why)) => why,
        other => panic!("{other:?}"),
    };
    assert!(
        told(store(&carol).read())
            .starts_with("its format marker (format/board) is at least 1572865 bytes,"),
    );
    assert!(
        told(store(&repos.bob).read())
            .starts_with("its format marker (format/board) is 2097152 bytes,"),
    );
}

#[path = "store_partial_state_tests.rs"]
mod state;

/// A fence's `MOVED` past every cap, in a clone made without blobs, is
/// refused by the least size the clone knows, never fetched (#2705 B13).
#[test]
fn a_moved_left_on_the_remote_is_told_at_least_its_size_unfetched() {
    let repos = fresh();
    let remote = repos.remote();
    let moved = blob_in(&remote, &vec![b'x'; 2 * 1024 * 1024]);
    let marker = blob_in(&remote, b"quecto-board 4\n");
    let format = raw_rows(
        &remote,
        &[
            ("100644", "MOVED", moved.clone()),
            ("100644", "board", marker),
        ],
    );
    let root = raw_rows(
        &remote,
        &[
            ("100644", "README.md", blob_in(&remote, README)),
            ("40000", "format", format),
            ("40000", "tasks", raw_rows(&remote, &[])),
        ],
    );
    let head = raw_commit(&remote, &root);
    sh(&remote, &["update-ref", BOARD_REF, &head.0]);
    let carol = partial(&repos, "blob:none", "carol");
    let read = store(&carol).read();
    assert!(
        matches!(&read, Err(BoardStoreError::MovedMalformed(why))
            if why.contains("(format/MOVED is at least 1572865 bytes; the most read is 4096)")),
        "{read:?}"
    );
    assert!(!held(&carol, &moved), "MOVED was fetched");
}
