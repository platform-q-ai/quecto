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

/// `name`, a clone of the remote made with `filter`, whose remote serves
/// filtered fetches and objects by id.
fn partial(repos: &Repos, filter: &str, name: &str) -> PathBuf {
    let remote = repos.remote();
    for (key, value) in [
        ("uploadpack.allowFilter", "true"),
        ("uploadpack.allowAnySHA1InWant", "true"),
    ] {
        sh(&remote, &["config", key, value]);
    }
    let url = format!("file://{}", remote.display());
    let filter = format!("--filter={filter}");
    sh(repos.dir.path(), &["clone", "-q", &filter, &url, name]);
    repos.dir.path().join(name)
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
